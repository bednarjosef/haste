package haste.testmod;

import net.fabricmc.api.ModInitializer;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

public class HasteTestMod implements ModInitializer {
    public static final Logger LOGGER = LoggerFactory.getLogger("haste-test");

    @Override
    public void onInitialize() {
        LOGGER.info("haste-test mod initialised");
    }
}
