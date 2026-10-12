package haste.recorder.client;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.ClientInput;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.world.entity.player.Input;

/**
 * Per-tick driver for the client recorder (registered from {@link HasteClientRecorder}).
 *
 * RECORDING TRACES: the player's input object is swapped for a {@link ScriptedInput} that
 * replays each scripted sequence (walk, sprint, jump, sprint-jump, stop, sneak,
 * walk-into-wall, step-up-slab, swim, fall — 100 ticks each); every tick the player's
 * pos/velocity/on_ground/pose/dimensions plus the scripted input are appended to
 * {@code cache/oracle/26.3/traces/<name>-<sequence>.json} (FORMATS.md).
 */
public final class RecorderState {
    private RecorderState() {}

    /** Set by a command, consumed on the next client tick. */
    public static java.util.function.Consumer<net.minecraft.commands.CommandSourceStack> pendingAction;

    private static TraceRecording activeTrace = null;
    private static BenchRecording activeBench = null;

    /** The unattended scenario (-Dhaste.scenario), if any. */
    static Scenario scenario = null;

    /** True while a trace recording runs (the scenario's record step waits on it). */
    public static boolean recording() {
        return activeTrace != null;
    }

    public static void tick(Minecraft mc) {
        if (scenario != null) scenario.tick(mc);
        if (pendingAction != null) {
            var action = pendingAction;
            pendingAction = null;
            action.accept(null);
        }
        if (activeTrace != null) {
            activeTrace.tick(mc);
            if (activeTrace.done()) activeTrace = null;
        }
        if (activeBench != null) {
            activeBench.tick();
            if (activeBench.done()) activeBench = null;
        }
    }

    // ---------------------------------------------------------------- commands

    public static void startRecording(String name) {
        Minecraft mc = Minecraft.getInstance();
        if (mc.player == null) {
            RecorderIO.chat(mc, "haste: join a world first");
            return;
        }
        if (activeTrace != null) {
            RecorderIO.chat(mc, "haste: a recording is already in progress (/haste stop)");
            return;
        }
        activeTrace = new TraceRecording(name);
        RecorderIO.chat(mc, "haste: recording 10 traces as '" + name + "-*' (hands off the keys!)");
    }

    public static void stopAll() {
        if (activeTrace != null) activeTrace.abort(Minecraft.getInstance());
        activeTrace = null;
        if (activeBench != null) activeBench.abort();
        activeBench = null;
        RecorderIO.chat(Minecraft.getInstance(), "haste: stopped");
    }

    public static void takeScreenshot(String name) {
        Minecraft mc = Minecraft.getInstance();
        if (mc.player == null) {
            RecorderIO.chat(mc, "haste: join a world first");
            return;
        }
        RecorderIO.captureScreenshot(name, err -> {});
    }

    public static void startBench(String name) {
        Minecraft mc = Minecraft.getInstance();
        if (mc.player == null) {
            RecorderIO.chat(mc, "haste: join a world first");
            return;
        }
        if (activeBench != null) {
            RecorderIO.chat(mc, "haste: a bench is already in progress");
            return;
        }
        activeBench = new BenchRecording(name);
        RecorderIO.chat(mc, "haste: benchmarking as '" + name + "' for 60 s");
    }

    // ---------------------------------------------------------------- trace recording

    /** One scripted input step: which keys are held, for how many ticks. */
    record Step(Input input, int ticks) {}

    static final class TraceRecording {
        final String baseName;
        final List<String> written = new ArrayList<>();
        final ScriptedInput scripted = new ScriptedInput();
        ClientInput savedInput = null;
        int sequenceIndex = 0;
        int tickInStep = 0;
        RecorderIO.TraceWriter writer = null;

        TraceRecording(String baseName) {
            this.baseName = baseName;
        }

        boolean done() {
            return sequenceIndex >= SEQUENCES.length && writer == null;
        }

        void abort(Minecraft mc) {
            if (writer != null) {
                writer.close();
                written.add(writer.fileName());
                writer = null;
            }
            restoreInput(mc);
        }

        void restoreInput(Minecraft mc) {
            if (savedInput != null && mc.player != null) {
                mc.player.input = savedInput;
                savedInput = null;
            }
        }

        void tick(Minecraft mc) {
            LocalPlayer player = mc.player;
            if (player == null) return; // left the world mid-recording; wait or abort manually

            // Install the scripted input once, keep it for the whole recording.
            if (savedInput == null) {
                savedInput = player.input;
                player.input = scripted;
            }

            if (writer == null) {
                if (sequenceIndex >= SEQUENCES.length) {
                    restoreInput(mc);
                    RecorderIO.chat(mc, "haste: done, " + written.size() + " traces in "
                        + RecorderIO.tracesDir());
                    return;
                }
                Sequence seq = SEQUENCES[sequenceIndex];
                scripted.set(seq.firstInput());
                tickInStep = 0;
                writer = new RecorderIO.TraceWriter(baseName + "-" + seq.name(), player);
                return;
            }

            Sequence seq = SEQUENCES[sequenceIndex];
            Step step = seq.stepAt(seq.indexOfStepForTick(tickInStep));
            scripted.set(step.input());

            // Record the player state at the START of this tick (FORMATS.md: pos is
            // start-of-tick; the recorded input is what this tick applies).
            var pos = player.position();
            var vel = player.getDeltaMovement();
            var dims = player.getDimensions(player.getPose());
            writer.write(
                step.input().forward() ? 1f : step.input().backward() ? -1f : 0f,
                step.input().left() ? 1f : step.input().right() ? -1f : 0f,
                step.input().jump(), step.input().shift(), step.input().sprint(),
                pos.x, pos.y, pos.z, vel.x, vel.y, vel.z,
                player.onGround(), player.getPose().getSerializedName(),
                dims.width(), dims.height());

            tickInStep++;
            if (tickInStep >= seq.totalTicks()) {
                writer.close();
                written.add(writer.fileName());
                writer = null;
                sequenceIndex++;
            }
        }
    }

    /** A named scripted sequence: inputs per tick. */
    record Sequence(String name, Step[] steps) {
        Input firstInput() {
            return steps[0].input();
        }

        int totalTicks() {
            int t = 0;
            for (Step s : steps) t += s.ticks();
            return t;
        }

        int indexOfStepForTick(int tick) {
            int acc = 0;
            for (int i = 0; i < steps.length; i++) {
                acc += steps[i].ticks();
                if (tick < acc) return i;
            }
            return steps.length - 1;
        }

        Step stepAt(int i) {
            return steps[i];
        }
    }

    static final Input WALK = new Input(true, false, false, false, false, false, false);
    static final Input SPRINT = new Input(true, false, false, false, false, false, true);
    static final Input JUMP = new Input(true, false, false, false, true, false, false);
    static final Input SPRINT_JUMP = new Input(true, false, false, false, true, false, true);
    static final Input SNEAK = new Input(true, false, false, false, false, true, false);
    static final Input STILL = new Input(false, false, false, false, false, false, false);

    static Step hold(Input in, int ticks) {
        return new Step(in, ticks);
    }

    /**
     * The scripted sequences, 100 ticks (5 s) each. Terrain-dependent sequences
     * (walk-into-wall, step-up-slab, swim, fall) expect the recording world from
     * docs/RECORDING.md; on plain ground they still run and record whatever actually happens.
     */
    static final Sequence[] SEQUENCES = {
        new Sequence("walk", new Step[] { hold(WALK, 100) }),
        new Sequence("sprint", new Step[] { hold(SPRINT, 100) }),
        new Sequence("jump", new Step[] { hold(JUMP, 100) }),
        new Sequence("sprint-jump", new Step[] { hold(SPRINT_JUMP, 100) }),
        new Sequence("stop", new Step[] { hold(STILL, 100) }),
        new Sequence("sneak", new Step[] { hold(SNEAK, 100) }),
        new Sequence("walk-into-wall", new Step[] { hold(WALK, 100) }),
        new Sequence("step-up-slab", new Step[] { hold(WALK, 100) }),
        // swim: forward only, jump held to stay up in water
        new Sequence("swim", new Step[] { hold(new Input(true, false, false, false, true, false, false), 100) }),
        new Sequence("fall", new Step[] { hold(STILL, 100) }),
    };

    // ---------------------------------------------------------------- bench recording

    static final class BenchRecording {
        final String name;
        final long startNanos = System.nanoTime();

        BenchRecording(String name) {
            this.name = name;
            RecorderIO.startBench(name);
        }

        boolean done() {
            return elapsed() >= 60.0;
        }

        double elapsed() {
            return (System.nanoTime() - startNanos) / 1e9;
        }

        void tick() {
            RecorderIO.benchTick();
        }

        void abort() {
            RecorderIO.finishBench(name);
        }
    }
}
