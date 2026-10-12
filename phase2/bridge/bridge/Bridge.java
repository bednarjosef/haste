package haste.bridge;

import java.io.InputStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.AnnotationNode;
import org.objectweb.asm.tree.ClassNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.FieldNode;
import org.objectweb.asm.tree.FrameNode;
import org.objectweb.asm.tree.IincInsnNode;
import org.objectweb.asm.tree.InsnList;
import org.objectweb.asm.tree.InsnNode;
import org.objectweb.asm.tree.LabelNode;
import org.objectweb.asm.tree.TryCatchBlockNode;
import org.objectweb.asm.tree.LdcInsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.TypeInsnNode;
import org.objectweb.asm.tree.VarInsnNode;

/**
 * Reads a mod's mixin classes with ASM, finds @Inject handlers + @Shadow fields, and generates for each
 * mixin a class in haste.gen with (a) the handler as a static method whose @Shadow accesses go through
 * RustState natives and (b) a boolean trampoline that builds the CallbackInfo, runs the handler and
 * reports whether ci.cancel() was called. Rust calls the trampolines at its landmarks.
 */
public final class Bridge implements Opcodes {
    static final String MIXIN = "Lorg/spongepowered/asm/mixin/Mixin;";
    static final String SHADOW = "Lorg/spongepowered/asm/mixin/Shadow;";
    static final String INJECT = "Lorg/spongepowered/asm/mixin/injection/Inject;";
    static final String CI = "org/spongepowered/asm/mixin/injection/callback/CallbackInfo";
    static final String STATE = "haste/bridge/RustState";

    static final class GenLoader extends ClassLoader {
        GenLoader() { super(Bridge.class.getClassLoader()); }
        Class<?> define(String name, byte[] b) { return defineClass(name, b, 0, b.length); }
    }
    static final GenLoader LOADER = new GenLoader();
    static final Map<String, Class<?>> GENERATED = new HashMap<>();

    /** Used by Rust to get a jclass for a generated class (FindClass cannot see our loader). */
    public static Class<?> classFor(String name) { return GENERATED.get(name); }

    /** Empty static method: the raw JNI call floor for the benchmark. */
    public static void noop() {}

    /**
     * @param mixinClasses fully-qualified mixin class names (from the mixin config, resolved by Rust)
     * @param stateFields  names of the Rust-owned fields; the index is the id passed to RustState
     * @return one line per handler: target|method|at|priority|cancellable|genClass|trampoline|mixin.handler
     */
    public static String scan(String[] mixinClasses, String[] stateFields) throws Exception {
        Map<String, Integer> fieldIds = new HashMap<>();
        for (int i = 0; i < stateFields.length; i++) fieldIds.put(stateFields[i], i);
        StringBuilder out = new StringBuilder();
        for (String name : mixinClasses) out.append(process(name, fieldIds));
        return out.toString();
    }

    static String process(String mixinName, Map<String, Integer> fieldIds) throws Exception {
        ClassNode cn = new ClassNode();
        try (InputStream in = Bridge.class.getClassLoader().getResourceAsStream(mixinName.replace('.', '/') + ".class")) {
            if (in == null) throw new IllegalArgumentException("mixin class not found on classpath: " + mixinName);
            new ClassReader(in).accept(cn, 0);
        }
        AnnotationNode mixin = find(cn.visibleAnnotations, MIXIN);
        if (mixin == null) mixin = find(cn.invisibleAnnotations, MIXIN);
        if (mixin == null) throw new IllegalStateException(mixinName + " has no @Mixin");
        @SuppressWarnings("unchecked")
        List<Type> targets = (List<Type>) value(mixin, "value", List.of());
        int priority = (Integer) value(mixin, "priority", 1000);
        if (targets.size() != 1) throw new UnsupportedOperationException("spike supports exactly one target");
        String target = targets.get(0).getClassName();

        // @Shadow fields -> Rust state ids
        Map<String, Integer> shadow = new HashMap<>();
        Map<String, String> shadowDesc = new HashMap<>();
        for (FieldNode f : cn.fields) {
            if (find(f.visibleAnnotations, SHADOW) == null) continue;
            if (!"I".equals(f.desc) && !f.desc.startsWith("L")) throw new UnsupportedOperationException("spike bridges int and object shadows only: " + f.name + f.desc);
            Integer id = fieldIds.get(f.name);
            if (id == null) throw new IllegalStateException("@Shadow " + f.name + " has no Rust-owned counterpart in " + target);
            shadow.put(f.name, id);
            shadowDesc.put(f.name, f.desc);
        }

        String genName = "haste/gen/" + cn.name.substring(cn.name.lastIndexOf('/') + 1) + "$Bridge";
        ClassNode gen = new ClassNode();
        gen.visit(V17, ACC_PUBLIC | ACC_FINAL | ACC_SUPER, genName, null, "java/lang/Object", null);
        StringBuilder out = new StringBuilder();

        for (MethodNode m : cn.methods) {
            AnnotationNode inject = find(m.visibleAnnotations, INJECT);
            if (inject == null) continue;
            @SuppressWarnings("unchecked")
            List<String> methods = (List<String>) value(inject, "method", List.of());
            @SuppressWarnings("unchecked")
            List<AnnotationNode> ats = (List<AnnotationNode>) value(inject, "at", List.of());
            boolean cancellable = (Boolean) value(inject, "cancellable", false);
            if (methods.size() != 1 || ats.size() != 1) throw new UnsupportedOperationException("spike: one method + one @At per @Inject");
            String at = (String) value(ats.get(0), "value", "HEAD");
            if (!m.desc.equals("(L" + CI + ";)V")) throw new UnsupportedOperationException("spike: handler must be (CallbackInfo)V, got " + m.desc);

            MethodNode h = rewriteHandler(cn.name, m, shadow, shadowDesc);
            gen.methods.add(h);
            String tramp = m.name + "$dispatch";
            gen.methods.add(trampoline(genName, h.name, tramp, methods.get(0), cancellable));
            out.append(String.join("|", target, methods.get(0), at, Integer.toString(priority),
                Boolean.toString(cancellable), genName.replace('/', '.'), tramp, mixinName + "." + m.name)).append('\n');
        }

        ClassWriter cw = new ClassWriter(ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS) {
            @Override protected String getCommonSuperClass(String a, String b) { return "java/lang/Object"; }
        };
        gen.accept(cw);
        String dotted = genName.replace('/', '.');
        GENERATED.put(dotted, LOADER.define(dotted, cw.toByteArray()));
        return out.toString();
    }

    /** Instance handler -> static method. `this` is only ever consumed by the shadow accesses we replace. */
    static MethodNode rewriteHandler(String mixinInternal, MethodNode m, Map<String, Integer> shadow, Map<String, String> shadowDesc) {
        MethodNode h = new MethodNode(ACC_PUBLIC | ACC_STATIC, m.name, m.desc, null, null);
        InsnList insns = new InsnList();
        Map<LabelNode, LabelNode> labels = new HashMap<>();
        for (AbstractInsnNode insn = m.instructions.getFirst(); insn != null; insn = insn.getNext())
            if (insn instanceof LabelNode l) labels.put(l, new LabelNode());
        for (AbstractInsnNode insn = m.instructions.getFirst(); insn != null; insn = insn.getNext()) {
            switch (insn) {
                case FrameNode f -> {}
                case VarInsnNode v when v.var == 0 -> {
                    if (v.getOpcode() != ALOAD) throw new UnsupportedOperationException("handler writes `this`");
                    insns.add(new InsnNode(ACONST_NULL));
                }
                case VarInsnNode v -> insns.add(new VarInsnNode(v.getOpcode(), v.var - 1));
                case IincInsnNode i -> insns.add(new IincInsnNode(i.var - 1, i.incr));
                case FieldInsnNode f when f.owner.equals(mixinInternal) -> {
                    Integer id = shadow.get(f.name);
                    if (id == null) throw new UnsupportedOperationException("non-shadow field access " + f.name);
                    boolean isInt = "I".equals(shadowDesc.get(f.name));
                    if (f.getOpcode() == GETFIELD) {          // [this] -> [value]
                        insns.add(new InsnNode(POP));
                        insns.add(new LdcInsnNode(id));
                        if (isInt) insns.add(new MethodInsnNode(INVOKESTATIC, STATE, "getInt", "(I)I", false));
                        else {
                            insns.add(new MethodInsnNode(INVOKESTATIC, STATE, "getObject", "(I)Ljava/lang/Object;", false));
                            insns.add(new TypeInsnNode(CHECKCAST, Type.getType(f.desc).getInternalName()));
                        }
                    } else if (f.getOpcode() == PUTFIELD) {   // [this, value] -> [this, value, id] -> [this, id, value] -> [this] -> []
                        insns.add(new LdcInsnNode(id));
                        insns.add(new InsnNode(SWAP));
                        if (isInt) insns.add(new MethodInsnNode(INVOKESTATIC, STATE, "setInt", "(II)V", false));
                        else insns.add(new MethodInsnNode(INVOKESTATIC, STATE, "setObject", "(ILjava/lang/Object;)V", false));
                        insns.add(new InsnNode(POP));
                    } else throw new UnsupportedOperationException("static shadow field");
                }
                case MethodInsnNode mi when mi.owner.equals(mixinInternal) ->
                    throw new UnsupportedOperationException("spike: handler calls another mixin method " + mi.name);
                case TypeInsnNode t when t.desc.equals(mixinInternal) ->
                    throw new UnsupportedOperationException("spike: handler references mixin type");
                default -> insns.add(insn.clone(labels));
            }
        }
        h.instructions = insns;
        h.tryCatchBlocks = new ArrayList<>();
        for (TryCatchBlockNode t : m.tryCatchBlocks)
            h.tryCatchBlocks.add(new TryCatchBlockNode(labels.get(t.start), labels.get(t.end), labels.get(t.handler), t.type));
        return h;
    }

    /** static boolean name() { CallbackInfo ci = new CallbackInfo(target, cancellable); handler(ci); return ci.isCancelled(); } */
    static MethodNode trampoline(String owner, String handler, String name, String targetMethod, boolean cancellable) {
        MethodNode t = new MethodNode(ACC_PUBLIC | ACC_STATIC, name, "()Z", null, null);
        InsnList l = t.instructions;
        l.add(new TypeInsnNode(NEW, CI));
        l.add(new InsnNode(DUP));
        l.add(new LdcInsnNode(targetMethod));
        l.add(new InsnNode(cancellable ? ICONST_1 : ICONST_0));
        l.add(new MethodInsnNode(INVOKESPECIAL, CI, "<init>", "(Ljava/lang/String;Z)V", false));
        l.add(new VarInsnNode(ASTORE, 0));
        l.add(new VarInsnNode(ALOAD, 0));
        l.add(new MethodInsnNode(INVOKESTATIC, owner, handler, "(L" + CI + ";)V", false));
        l.add(new VarInsnNode(ALOAD, 0));
        l.add(new MethodInsnNode(INVOKEVIRTUAL, CI, "isCancelled", "()Z", false));
        l.add(new InsnNode(IRETURN));
        return t;
    }

    static AnnotationNode find(List<AnnotationNode> list, String desc) {
        if (list == null) return null;
        for (AnnotationNode a : list) if (desc.equals(a.desc)) return a;
        return null;
    }

    static Object value(AnnotationNode a, String key, Object dflt) {
        if (a.values == null) return dflt;
        for (int i = 0; i < a.values.size(); i += 2) if (key.equals(a.values.get(i))) return a.values.get(i + 1);
        return dflt;
    }
}
