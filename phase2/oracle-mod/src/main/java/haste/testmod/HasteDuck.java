package haste.testmod;

/** "Duck" interface a mixin adds to LocalPlayer so other mod code (and Rust) can reach the @Unique field. */
public interface HasteDuck {
    int haste$getScore();
    void haste$setScore(int v);
}
