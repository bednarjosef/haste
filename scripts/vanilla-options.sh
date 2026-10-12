#!/usr/bin/env bash
# Writes the fixed options.txt for scripted vanilla reference runs (scripts/vanilla-client.sh).
# Every run starts from exactly these settings; keys not listed take the game's defaults.
# usage: scripts/vanilla-options.sh <options.txt> <width> <height> [key:value ...]
#   extra key:value pairs override or add lines (e.g. renderDistance:12)
# Key names are the 26.3 options.txt keys (version 5023), observed from a fresh game dir.
set -euo pipefail
OUT=${1:?options.txt path}; WIDTH=${2:?width}; HEIGHT=${3:?height}; shift 3
declare -A opt=(
  [version]=5023
  # the picture: what the screenshot comparison depends on
  [renderDistance]=8 [simulationDistance]=8
  [fov]=0.0                       # stored as (fov-70)/40: 0.0 = 70 degrees
  [fovEffectScale]=0.0            # no sprint/fly FOV change
  [guiScale]=2
  [graphicsPreset]='"custom"'   # a named preset overwrites the values below on load
  [ao]=true                       # smooth lighting
  [biomeBlendRadius]=2
  [mipmapLevels]=4
  [gamma]=0.5
  [renderClouds]='"false"'        # clouds drift with time: not reproducible
  [chunkSectionFadeInTime]=0.0    # new sections appear at once, not faded in
  [bobView]=false
  [entityShadows]=true
  [particles]=0
  [vignette]=true
  [cutoutLeaves]=true
  [improvedTransparency]=false
  [enableVsync]=false [maxFps]=120
  [inactivityFpsLimit]='"minimized"'
  [fullscreen]=false
  [overrideWidth]="$WIDTH" [overrideHeight]="$HEIGHT"
  # nothing may pop up, pause or wait for a human
  [pauseOnLostFocus]=false
  [onboardAccessibility]=false
  [skipMultiplayerWarning]=true
  [joinedFirstServer]=true
  [tutorialStep]=none
  [narrator]=0 [narratorHotkey]=false
  [realmsNotifications]=false
  [inGameNotification]=false
  [musicToast]='"never"'
  [telemetryOptInExtra]=false
  [hideSplashTexts]=true
  [showAutosaveIndicator]=false
  [chatLinksPrompt]=false
  [autoJump]=false
  [lang]=en_us
  [soundCategory_master]=0.0      # silent: runs on the user's desktop
)
for kv in "$@"; do opt[${kv%%:*}]=${kv#*:}; done
mkdir -p "$(dirname "$OUT")"
{
  for k in "${!opt[@]}"; do printf '%s:%s\n' "$k" "${opt[$k]}"; done
} | sort > "$OUT"
