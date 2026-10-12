package haste.surface;

import java.io.*;
import java.nio.file.*;
import java.util.*;
import java.util.stream.*;
import java.util.zip.*;
import org.objectweb.asm.*;
import org.objectweb.asm.tree.*;

/**
 * Step 1: measure the vanilla surface Fabric mods touch. Runs every .class in each mod jar (and nested
 * META-INF/jars) through ASM, records references to net/minecraft and com/mojang, and writes reports.
 * usage: Scan <mods dir> <out dir>
 */
public class Scan {
    static final String MIXIN = "Lorg/spongepowered/asm/mixin/Mixin;";
    static final String UNIQUE = "Lorg/spongepowered/asm/mixin/Unique;";

    static boolean vanilla(String internal) {
        return internal != null && (internal.startsWith("net/minecraft/") || internal.startsWith("com/mojang/"));
    }

    /** Per-mod sets. */
    static final class Mod {
        final String name;
        final Set<String> classes = new TreeSet<>(), methods = new TreeSet<>(), fields = new TreeSet<>();
        final Set<String> extendsOrImplements = new TreeSet<>(), mixinTargets = new TreeSet<>();
        final Set<String> uniqueFields = new TreeSet<>(), mixinInterfaces = new TreeSet<>();
        int modClasses, mixinClasses;
        Mod(String n) { name = n; }
    }

    public static void main(String[] args) throws Exception {
        Path modsDir = Path.of(args[0]), out = Path.of(args[1]);
        Files.createDirectories(out);
        List<Mod> mods = new ArrayList<>();
        try (Stream<Path> s = Files.list(modsDir)) {
            for (Path jar : s.filter(p -> p.toString().endsWith(".jar")).sorted().toList()) {
                Mod m = new Mod(jar.getFileName().toString().replace(".jar", ""));
                try (InputStream in = Files.newInputStream(jar)) { scanJar(in, m, 0); }
                mods.add(m);
                System.out.printf("%-40s classes=%5d mixins=%4d vanillaClasses=%4d methods=%5d fields=%4d%n",
                    m.name, m.modClasses, m.mixinClasses, m.classes.size(), m.methods.size(), m.fields.size());
            }
        }
        // aggregate
        Map<String, Integer> classUse = new HashMap<>(), methodUse = new HashMap<>(), fieldUse = new HashMap<>();
        Map<String, Integer> superUse = new HashMap<>(), targetUse = new HashMap<>();
        for (Mod m : mods) {
            m.classes.forEach(c -> classUse.merge(c, 1, Integer::sum));
            m.methods.forEach(c -> methodUse.merge(c, 1, Integer::sum));
            m.fields.forEach(c -> fieldUse.merge(c, 1, Integer::sum));
            m.extendsOrImplements.forEach(c -> superUse.merge(c, 1, Integer::sum));
            m.mixinTargets.forEach(c -> targetUse.merge(c, 1, Integer::sum));
        }
        write(out.resolve("classes.txt"), classUse);
        write(out.resolve("methods.txt"), methodUse);
        write(out.resolve("fields.txt"), fieldUse);
        write(out.resolve("subclassed.txt"), superUse);
        write(out.resolve("mixin-targets.txt"), targetUse);
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("per-mod.txt")))) {
            w.println("mod\tmodClasses\tmixinClasses\tvanillaClasses\tvanillaMethods\tvanillaFields\tsubclassed\tmixinTargets\tuniqueFields\tmixinIfaces");
            for (Mod m : mods)
                w.printf("%s\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d%n", m.name, m.modClasses, m.mixinClasses, m.classes.size(),
                    m.methods.size(), m.fields.size(), m.extendsOrImplements.size(), m.mixinTargets.size(),
                    m.uniqueFields.size(), m.mixinInterfaces.size());
            for (Mod m : mods) {
                w.println("\n== " + m.name + " subclasses/implements: " + m.extendsOrImplements);
                w.println("== " + m.name + " mixin targets: " + m.mixinTargets);
                w.println("== " + m.name + " @Unique fields: " + m.uniqueFields);
                w.println("== " + m.name + " interfaces added by mixins: " + m.mixinInterfaces);
            }
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("summary.txt")))) {
            Set<String> allM = new HashSet<>(), allF = new HashSet<>(), allC = new HashSet<>();
            int[] n = {0};
            for (Mod m : mods) { allM.addAll(m.methods); allF.addAll(m.fields); allC.addAll(m.classes); n[0] += m.uniqueFields.size(); }
            w.printf("mods=%d%ndistinct vanilla classes=%d%ndistinct vanilla methods=%d%ndistinct vanilla fields=%d%n",
                mods.size(), allC.size(), allM.size(), allF.size());
            w.printf("vanilla types subclassed/implemented (distinct)=%d%nmixin target classes (distinct)=%d%n@Unique fields total=%d%n",
                superUse.size(), targetUse.size(), n[0]);
            // classes used by >= half the mods
            long common = classUse.values().stream().filter(v -> v >= mods.size() / 2).count();
            w.printf("vanilla classes used by >= %d mods=%d%n", mods.size() / 2, common);
            w.println("\n== top 50 vanilla methods (mods using) ==");
            top(methodUse, 50).forEach(e -> w.printf("%3d  %s%n", e.getValue(), e.getKey()));
            w.println("\n== top 30 vanilla types subclassed/implemented (mods) ==");
            top(superUse, 30).forEach(e -> w.printf("%3d  %s%n", e.getValue(), e.getKey()));
            w.println("\n== top 30 mixin targets (mods) ==");
            top(targetUse, 30).forEach(e -> w.printf("%3d  %s%n", e.getValue(), e.getKey()));
            w.println("\n== top 30 vanilla classes (mods) ==");
            top(classUse, 30).forEach(e -> w.printf("%3d  %s%n", e.getValue(), e.getKey()));
        }
        System.out.println(Files.readString(out.resolve("summary.txt")));
    }

    static List<Map.Entry<String, Integer>> top(Map<String, Integer> m, int n) {
        return m.entrySet().stream()
            .sorted(Map.Entry.<String, Integer>comparingByValue().reversed().thenComparing(Map.Entry.comparingByKey()))
            .limit(n).toList();
    }

    static void write(Path p, Map<String, Integer> m) throws IOException {
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(p))) {
            top(m, Integer.MAX_VALUE).forEach(e -> w.printf("%d\t%s%n", e.getValue(), e.getKey()));
        }
    }

    static void scanJar(InputStream raw, Mod m, int depth) throws IOException {
        ZipInputStream z = new ZipInputStream(raw);
        for (ZipEntry e; (e = z.getNextEntry()) != null; ) {
            if (e.getName().endsWith(".class") && !e.getName().startsWith("META-INF/")) {
                byte[] b = z.readAllBytes();
                try { scanClass(b, m); } catch (Exception ex) { System.err.println("skip " + e.getName() + ": " + ex); }
            } else if (e.getName().startsWith("META-INF/jars/") && e.getName().endsWith(".jar") && depth < 3) {
                scanJar(new ByteArrayInputStream(z.readAllBytes()), m, depth + 1);
            }
        }
    }

    static void scanClass(byte[] b, Mod m) {
        ClassNode cn = new ClassNode();
        new ClassReader(b).accept(cn, ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES);
        if (vanilla(cn.name)) return; // shaded vanilla copies are not mod code
        m.modClasses++;
        AnnotationNode mixin = find(cn.invisibleAnnotations, MIXIN);
        if (mixin == null) mixin = find(cn.visibleAnnotations, MIXIN);
        if (mixin != null) {
            m.mixinClasses++;
            Object v = value(mixin, "value");
            if (v instanceof List<?> l) for (Object t : l) if (t instanceof Type ty) { m.mixinTargets.add(ty.getInternalName()); m.classes.add(ty.getInternalName()); }
            Object tg = value(mixin, "targets");
            if (tg instanceof List<?> l) for (Object t : l) { String s = t.toString().replace('.', '/'); m.mixinTargets.add(s); m.classes.add(s); }
            for (FieldNode f : cn.fields)
                if (find(f.visibleAnnotations, UNIQUE) != null || find(f.invisibleAnnotations, UNIQUE) != null) m.uniqueFields.add(cn.name + "." + f.name);
            if (cn.interfaces != null) for (String i : cn.interfaces) if (!vanilla(i)) m.mixinInterfaces.add(i);
        } else {
            if (vanilla(cn.superName)) { m.extendsOrImplements.add(cn.superName); m.classes.add(cn.superName); }
            if (cn.interfaces != null) for (String i : cn.interfaces) if (vanilla(i)) { m.extendsOrImplements.add(i); m.classes.add(i); }
        }
        for (MethodNode mn : cn.methods) {
            if (mn.instructions == null) continue;
            for (AbstractInsnNode insn = mn.instructions.getFirst(); insn != null; insn = insn.getNext()) {
                if (insn instanceof MethodInsnNode mi && vanilla(mi.owner)) {
                    m.classes.add(mi.owner); m.methods.add(mi.owner + "." + mi.name + mi.desc);
                } else if (insn instanceof FieldInsnNode fi && vanilla(fi.owner)) {
                    m.classes.add(fi.owner); m.fields.add(fi.owner + "." + fi.name);
                } else if (insn instanceof TypeInsnNode ti && vanilla(ti.desc)) {
                    m.classes.add(ti.desc);
                } else if (insn instanceof InvokeDynamicInsnNode id) {
                    for (Object a : id.bsmArgs) if (a instanceof Handle h && vanilla(h.getOwner())) { m.classes.add(h.getOwner()); m.methods.add(h.getOwner() + "." + h.getName() + h.getDesc()); }
                }
            }
        }
    }

    static AnnotationNode find(List<AnnotationNode> l, String desc) {
        if (l == null) return null;
        for (AnnotationNode a : l) if (desc.equals(a.desc)) return a;
        return null;
    }
    static Object value(AnnotationNode a, String k) {
        if (a.values == null) return null;
        for (int i = 0; i < a.values.size(); i += 2) if (k.equals(a.values.get(i))) return a.values.get(i + 1);
        return null;
    }
}
