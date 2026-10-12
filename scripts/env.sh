# Source this for a dev shell: cache location, JDK, and the project-local Vulkan driver.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export HASTE_ROOT="$ROOT"
export HASTE_CACHE="$ROOT/cache"
export HASTE_WT="${HASTE_WT:-$(dirname "$ROOT")/haste-wt}"
export JAVA_HOME="$ROOT/cache/jdk"
export PATH="$JAVA_HOME/bin:$PATH"
# Machines without a system Vulkan ICD can point HASTE_VULKAN_ICD at a locally built driver's
# ICD json (set it in your shell profile, never in the repo). Harmless elsewhere.
LOCAL_ICD=${HASTE_VULKAN_ICD:-}
[ -n "$LOCAL_ICD" ] && [ -f "$LOCAL_ICD" ] && [ -z "${VK_DRIVER_FILES:-}" ] && export VK_DRIVER_FILES="$LOCAL_ICD"
