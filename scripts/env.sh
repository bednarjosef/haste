# Source this for a dev shell: cache location, JDK, and the project-local Vulkan driver.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export HASTE_CACHE="$ROOT/cache"
export JAVA_HOME="$ROOT/cache/jdk"
export PATH="$JAVA_HOME/bin:$PATH"
# This machine has no system Vulkan ICD; fastcraft built a local RADV. Harmless elsewhere.
LOCAL_ICD=${HASTE_VULKAN_ICD:-}
[ -f "$LOCAL_ICD" ] && [ -z "${VK_DRIVER_FILES:-}" ] && export VK_DRIVER_FILES="$LOCAL_ICD"
