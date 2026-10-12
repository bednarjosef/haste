package haste.gui;

/** Game-side entry point: Rust holds the one instance and calls its methods. */
public final class Bridge {
    public static Bridge create() { return new Bridge(); }
}
