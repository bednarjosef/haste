package haste.recorder;

import net.fabricmc.api.ModInitializer;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/** Entry point. The actual dumping is driven from {@link ServerDumps} via a mixin on tickServer. */
public class HasteRecorderMod implements ModInitializer {
    public static final Logger LOGGER = LoggerFactory.getLogger("haste-recorder");

    @Override
    public void onInitialize() {
        LOGGER.info("haste-recorder initialised");
    }
}
