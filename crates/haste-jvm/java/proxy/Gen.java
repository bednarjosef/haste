package haste.proxy;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;
import org.objectweb.asm.commons.MethodRemapper;
import org.objectweb.asm.commons.SimpleRemapper;
import org.objectweb.asm.tree.*;

/**
 * Proxy generator. Reads vanilla classes from the (cached, never shipped) 26.3 client jar and writes
 * same-name, same-signature classes whose bodies are: native (Rust-backed), a field getter (mirror),
 * a static delegate (Java-side batching), or a stub that throws + logs. Mixins from the mod jar that
 * target a generated class get their @Unique fields, interfaces and plain methods merged in.
 *
 * usage: Gen <client.jar> <mod.jar> <config> <outDir>
 * config lines:  class <internal>            generate this class (+ transitive supers/interfaces)
 *                native <owner>#<name><desc>
 *                getter <owner>#<name><desc>=<field>[:<desc>]   (field is added if missing)
 *                delegate <owner>#<name><desc>=<class>#<method>
 *                mixin <internal mixin class>
 */
public class Gen implements Opcodes {
    static ZipFile client, mod;
    static final Map<String, String> impl = new HashMap<>();     // owner#name+desc -> "native" | "getter=..." | "delegate=..."
    static final Map<String, List<ClassNode>> mixinsByTarget = new HashMap<>();
    static final Map<String, ClassNode> cache = new HashMap<>();
    static int classes, methods, natives, getters, delegates, stubs, merged;

    static boolean vanilla(String n) { return n != null && (n.startsWith("net/minecraft/") || n.startsWith("com/mojang/")); }

    public static void main(String[] a) throws Exception {
        client = new ZipFile(a[0]); mod = new ZipFile(a[1]); Path out = Path.of(a[3]);
        Set<String> roots = new LinkedHashSet<>();
        for (String line : Files.readAllLines(Path.of(a[2]))) {
            line = line.trim();
            if (line.isEmpty() || line.startsWith("#")) continue;
            String[] p = line.split("\\s+", 2);
            switch (p[0]) {
                case "class" -> roots.add(p[1]);
                case "native" -> impl.put(p[1], "native");
                case "getter", "delegate" -> { int eq = p[1].indexOf('='); impl.put(p[1].substring(0, eq), p[0] + "=" + p[1].substring(eq + 1)); }
                case "mixin" -> {
                    ClassNode mx = read(mod, p[1]);
                    AnnotationNode an = find(mx.invisibleAnnotations, "Lorg/spongepowered/asm/mixin/Mixin;"); // @Mixin has CLASS retention
                    if (an == null) an = find(mx.visibleAnnotations, "Lorg/spongepowered/asm/mixin/Mixin;");
                    if (an == null) throw new IllegalStateException("no @Mixin on " + p[1] + " visible=" + mx.visibleAnnotations + " invisible=" + mx.invisibleAnnotations);
                    for (Type t : (List<Type>) value(an, "value", List.of())) mixinsByTarget.computeIfAbsent(t.getInternalName(), k -> new ArrayList<>()).add(mx);
                }
                default -> throw new IllegalArgumentException(line);
            }
        }
        // closure over superclasses and interfaces
        Deque<String> todo = new ArrayDeque<>(roots);
        Set<String> all = new LinkedHashSet<>();
        while (!todo.isEmpty()) {
            String n = todo.poll();
            if (!vanilla(n) || !all.add(n)) continue;
            ClassNode cn = read(client, n);
            todo.add(cn.superName);
            if (cn.interfaces != null) todo.addAll(cn.interfaces);
        }
        for (String n : all) generate(read(client, n), out);
        System.out.printf("generated %d classes, %d methods: %d native, %d getter, %d delegate, %d stub; merged %d mixin members%n",
            classes, methods, natives, getters, delegates, stubs, merged);
    }

    static ClassNode read(ZipFile z, String internal) throws Exception {
        String key = z.getName() + "!" + internal;
        ClassNode cn = cache.get(key);
        if (cn != null) return cn;
        ZipEntry e = z.getEntry(internal + ".class");
        if (e == null) throw new IllegalArgumentException("not in " + z.getName() + ": " + internal);
        try (InputStream in = z.getInputStream(e)) { cn = new ClassNode(); new ClassReader(in).accept(cn, 0); }
        cache.put(key, cn);
        return cn;
    }

    static void generate(ClassNode src, Path out) throws Exception {
        ClassNode g = new ClassNode();
        g.version = V21;
        g.access = src.access & ~(ACC_RECORD);
        g.name = src.name; g.superName = src.superName; g.interfaces = new ArrayList<>(src.interfaces);
        boolean iface = (src.access & ACC_INTERFACE) != 0;
        if (!vanilla(src.superName) && !iface) g.fields.add(new FieldNode(ACC_PUBLIC, "haste$handle", "J", null, null));
        for (FieldNode f : src.fields) {
            int acc = (f.access & ACC_STATIC) != 0 ? f.access : f.access & ~ACC_FINAL;
            g.fields.add(new FieldNode(acc, f.name, f.desc, null, (f.access & ACC_STATIC) != 0 ? f.value : null));
        }
        boolean hasNoArg = false;
        for (MethodNode m : src.methods) {
            if (m.name.equals("<clinit>")) continue;
            MethodNode n = new MethodNode(m.access & ~(ACC_NATIVE | ACC_SYNCHRONIZED), m.name, m.desc, null, m.exceptions == null ? null : m.exceptions.toArray(new String[0]));
            methods++;
            if (m.name.equals("<init>")) { ctorBody(n, src); if (m.desc.equals("()V")) hasNoArg = true; }
            else if ((m.access & ACC_ABSTRACT) != 0) { /* stays abstract */ }
            else body(g, n, src.name + "#" + m.name + m.desc);
            g.methods.add(n);
        }
        if (!iface && !hasNoArg) { MethodNode n = new MethodNode(ACC_PUBLIC, "<init>", "()V", null, null); ctorBody(n, src); g.methods.add(n); }
        for (ClassNode mx : mixinsByTarget.getOrDefault(src.name, List.of())) merge(g, mx);
        ClassWriter cw = new ClassWriter(ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS) {
            @Override protected String getCommonSuperClass(String x, String y) { return "java/lang/Object"; }
        };
        g.accept(cw);
        Path p = out.resolve(src.name + ".class");
        Files.createDirectories(p.getParent());
        Files.write(p, cw.toByteArray());
        classes++;
    }

    /** Every generated constructor ignores its arguments and chains to the superclass no-arg constructor. */
    static void ctorBody(MethodNode n, ClassNode src) {
        InsnList l = n.instructions;
        l.add(new VarInsnNode(ALOAD, 0));
        if ("java/lang/Enum".equals(src.superName)) {
            l.add(new LdcInsnNode("haste")); l.add(new InsnNode(ICONST_0));
            l.add(new MethodInsnNode(INVOKESPECIAL, "java/lang/Enum", "<init>", "(Ljava/lang/String;I)V", false));
        } else l.add(new MethodInsnNode(INVOKESPECIAL, src.superName, "<init>", "()V", false));
        l.add(new InsnNode(RETURN));
    }

    static void body(ClassNode g, MethodNode n, String key) {
        String how = impl.get(key);
        Type ret = Type.getReturnType(n.desc);
        InsnList l = n.instructions;
        if (how == null) {
            stubs++;
            l.add(new LdcInsnNode(key));
            l.add(new MethodInsnNode(INVOKESTATIC, "haste/rt/Unimplemented", "fail", "(Ljava/lang/String;)Ljava/lang/RuntimeException;", false));
            l.add(new InsnNode(ATHROW));
        } else if (how.equals("native")) {
            natives++; n.access |= ACC_NATIVE;
        } else if (how.startsWith("getter=")) {
            getters++;
            String spec = how.substring(7); String field = spec, desc = ret.getDescriptor();
            if (spec.contains(":")) { field = spec.substring(0, spec.indexOf(':')); desc = spec.substring(spec.indexOf(':') + 1); }
            final String fd = field;
            if (g.fields.stream().noneMatch(f -> f.name.equals(fd))) g.fields.add(new FieldNode(ACC_PUBLIC, field, desc, null, null));
            l.add(new VarInsnNode(ALOAD, 0));
            l.add(new FieldInsnNode(GETFIELD, g.name, field, desc));
            l.add(new InsnNode(ret.getOpcode(IRETURN)));
        } else if (how.startsWith("delegate=")) {
            delegates++;
            String[] t = how.substring(9).split("#");
            boolean isStatic = (n.access & ACC_STATIC) != 0;
            StringBuilder d = new StringBuilder("(");
            int slot = 0;
            if (!isStatic) { l.add(new VarInsnNode(ALOAD, 0)); d.append("Ljava/lang/Object;"); slot = 1; }
            for (Type at : Type.getArgumentTypes(n.desc)) {
                l.add(new VarInsnNode(at.getOpcode(ILOAD), slot)); slot += at.getSize();
                d.append(at.getSort() == Type.OBJECT || at.getSort() == Type.ARRAY ? "Ljava/lang/Object;" : at.getDescriptor());
            }
            d.append(")").append(ret.getSort() == Type.OBJECT || ret.getSort() == Type.ARRAY ? "Ljava/lang/Object;" : ret.getDescriptor());
            l.add(new MethodInsnNode(INVOKESTATIC, t[0], t[1], d.toString(), false));
            if (ret.getSort() == Type.OBJECT || ret.getSort() == Type.ARRAY) l.add(new TypeInsnNode(CHECKCAST, ret.getInternalName()));
            l.add(new InsnNode(ret.getOpcode(IRETURN)));
        }
    }

    /** @Unique fields, added interfaces and plain methods of a mixin become members of the proxy (like real Mixin does). */
    static void merge(ClassNode g, ClassNode mx) {
        SimpleRemapper rm = new SimpleRemapper(mx.name, g.name);
        for (String i : mx.interfaces) if (!g.interfaces.contains(i)) { g.interfaces.add(i); merged++; }
        for (FieldNode f : mx.fields) {
            if (find(f.visibleAnnotations, "Lorg/spongepowered/asm/mixin/Shadow;") != null || find(f.invisibleAnnotations, "Lorg/spongepowered/asm/mixin/Shadow;") != null) continue;
            g.fields.add(new FieldNode(f.access, f.name, f.desc, null, f.value)); merged++;
        }
        for (MethodNode m : mx.methods) {
            if (m.name.equals("<init>") || find(m.visibleAnnotations, "Lorg/spongepowered/asm/mixin/injection/Inject;") != null
                || find(m.invisibleAnnotations, "Lorg/spongepowered/asm/mixin/injection/Inject;") != null
                || find(m.visibleAnnotations, "Lorg/spongepowered/asm/mixin/Shadow;") != null
                || find(m.invisibleAnnotations, "Lorg/spongepowered/asm/mixin/Shadow;") != null) continue;
            MethodNode n = new MethodNode(m.access, m.name, m.desc, null, m.exceptions == null ? null : m.exceptions.toArray(new String[0]));
            m.accept(new MethodRemapper(n, rm));
            g.methods.add(n); merged++;
        }
    }

    static AnnotationNode find(List<AnnotationNode> l, String d) { if (l == null) return null; for (AnnotationNode a : l) if (d.equals(a.desc)) return a; return null; }
    static Object value(AnnotationNode a, String k, Object dflt) { if (a == null || a.values == null) return dflt; for (int i = 0; i < a.values.size(); i += 2) if (k.equals(a.values.get(i))) return a.values.get(i + 1); return dflt; }
}
