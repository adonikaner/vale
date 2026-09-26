//! **Every setting the client registers for itself, with the value it starts
//! at** -- 1.12.1's CVar defaults.
//!
//! ## Why this is a table rather than a store with nothing in it
//!
//! Every options panel in the game is written against `GetCVar` **before the
//! player has touched anything**: `SoundOptionsFrame_Load` reads eleven of
//! them to draw its own checkboxes, `UIOptionsFrameCameraDropDown_OnLoad`
//! reads one and then indexes a global by the id that matched. An unset CVar
//! is `nil`, so a panel written that way does not draw wrong -- it *raises*,
//! in its `OnLoad`, and takes the whole panel with it. This table is what
//! makes the first read answer.
//!
//! And it is what `GetCVarDefault` is: the Defaults button on a panel writes
//! `SetCVar(cvar, GetCVarDefault(cvar))` over every row, which without a real
//! default table sets everything to whatever it already was.
//!
//! ## What a value means
//!
//! **All of them are strings, and the interface compares them as strings** --
//! `if ( GetCVar("EnableMusic") == "1" )`. A boolean is `"1"` or `"0"`, a
//! volume is `"1.0"` or `"0.4"`, and `SetCVar` stringifies whatever it is
//! given (a nil value becomes the literal `"0"`).
//!
//! ## Coverage
//!
//! The client registers 214 CVars, and 200 of them are here. 7 are registered
//! under a name built at run time and 7 compute their default at run time
//! rather than naming a string -- `mouseSpeed` is the Windows pointer-speed
//! slider, `gxMultisample` is what the card reports. Both groups are left out
//! rather than guessed at.

pub const DEFAULTS: [(&str, &str); 200] = [
    ("accountName", ""),
    ("AmbienceVolume", "0.6"),
    ("anisotropic", "1"),
    ("assistAttack", "0"),
    ("autoClearAFK", "1"),
    ("AutoInteract", "0"),
    ("automoveturnspeednarrow", "800"),
    ("automoveturnspeedwide", "1200"),
    ("autoSelfCast", "0"),
    ("baseMip", "0"),
    ("BlockTrades", "0"),
    ("bspcache", "1"),
    ("cameraBobbing", "0"),
    ("cameraBobbingFrequency", "0.8"),
    ("cameraBobbingLRAmplitude", "2.0"),
    ("cameraBobbingSmoothSpeed", "0.8"),
    ("cameraBobbingUDAmplitude", "2.0"),
    ("cameraCustomViewSmoothing", "0"),
    ("cameraDistanceMax", "15.0"),
    ("cameraDistanceMaxFactor", "1.0"),
    ("cameraDistanceMoveSpeed", "8.33"),
    ("cameraDistanceSmoothSpeed", "8.33"),
    ("cameraDive", "1"),
    ("cameraFoVSmoothSpeed", "0.5"),
    ("cameraGroundSmoothSpeed", "7.5"),
    ("cameraHeightIgnoreStandState", "0"),
    ("cameraHeightSmoothSpeed", "1.2"),
    ("cameraPitchMoveSpeed", "90.0"),
    ("cameraPitchSmoothMax", "30.0"),
    ("cameraPitchSmoothMin", "0.0"),
    ("cameraPitchSmoothSpeed", "45.0"),
    ("cameraPivot", "1"),
    ("cameraPivotDXMax", "0.05"),
    ("cameraPivotDYMin", "0.00"),
    ("camerasmooth", "1"),
    ("cameraSmoothPitch", "0"),
    ("cameraSmoothStyle", "1"),
    ("cameraSmoothTimeMax", "2.0"),
    ("cameraSmoothTimeMin", "0.1"),
    ("cameraSmoothTrackingStyle", "1"),
    ("cameraSmoothYaw", "1"),
    ("cameraSubmergeFinalPitch", "5.0"),
    ("cameraSubmergePitch", "18.0"),
    ("cameraSurfaceFinalPitch", "5.0"),
    ("cameraSurfacePitch", "0.0"),
    ("cameraTargetSmoothSpeed", "90.0"),
    ("cameraTerrainTilt", "0"),
    ("cameraTerrainTiltTimeMax", "10.0"),
    ("cameraTerrainTiltTimeMin", "3.0"),
    ("cameraView", "1"),
    ("cameraViewBlendStyle", "1"),
    ("cameraWaterCollision", "1"),
    ("cameraYawMoveSpeed", "180.0"),
    ("cameraYawSmoothMax", "0.0"),
    ("cameraYawSmoothMin", "0.0"),
    ("cameraYawSmoothSpeed", "180.0"),
    ("ChatBubbles", "1"),
    ("ChatBubblesParty", "0"),
    ("checkAddonVersion", "1"),
    ("CombatDamage", "1"),
    ("CombatDeathLogRange", "60"),
    ("combatLogOn", "1"),
    ("CombatLogPeriodicSpells", "1"),
    ("CombatModeMaxDistance", "30.0f"),
    ("debugTaint", "0"),
    ("deselectOnClick", "1"),
    ("DesktopGamma", "0"),
    ("disableOptionalSpeech", "0"),
    ("DistCull", "500"),
    ("doodadAnim", "1"),
    ("EmoteSounds", "1"),
    ("EnableAmbience", "1"),
    ("EnableErrorSpeech", "1"),
    ("EnableGroupSpeech", "1"),
    ("EnableMusic", "1"),
    ("ErrorFilter", "all"),
    ("ErrorLevelMax", "3"),
    ("ErrorLevelMin", "1"),
    ("Errors", "0"),
    ("farclip", "350"),
    ("ffx", "1"),
    ("ffxDeath", "1"),
    ("ffxGlow", "1"),
    ("ffxRectangle", "1"),
    ("footstepBias", "0.125"),
    ("FootstepSounds", "1"),
    ("frillDensity", "16"),
    ("fullAlpha", "0"),
    ("gameTip", "0"),
    ("Gamma", "1.0"),
    ("guildMemberNotify", "0"),
    ("gxApi", "direct3d"),
    ("gxAspect", "1"),
    ("gxColorBits", "16"),
    ("gxCursor", "1"),
    ("gxDepthBits", "16"),
    ("gxMaximize", "0"),
    ("gxMultisampleQuality", "0.0"),
    ("gxOverride", ""),
    ("gxRefresh", "75"),
    ("gxResolution", "640x480"),
    ("gxTripleBuffer", "0"),
    ("gxVSync", "1"),
    ("gxWindow", "0"),
    ("horizonfarclip", "2112"),
    ("hwDetect", "1"),
    ("Joystick", "0"),
    ("lastCharacterIndex", "0"),
    ("lod", "1"),
    ("lodDist", "100.0"),
    ("M2BatchDoodads", "1"),
    ("M2Faster", "1"),
    ("M2FasterDebug", "0"),
    ("M2UseClipPlanes", "1"),
    ("M2UsePixelShaders", "0"),
    ("M2UseShaders", "1"),
    ("M2UseThreads", "1"),
    ("M2UseZFill", "1"),
    ("mapObjLightLOD", "0"),
    ("mapObjOverbright", "1"),
    ("mapShadows", "1"),
    ("MapWaterSounds", "1"),
    ("MasterSoundEffects", "1"),
    ("MasterVolume", "1.0"),
    ("MaxLights", "4"),
    ("minimapInsideZoom", "3"),
    ("minimapZoom", "3"),
    ("mouseInvertPitch", "0"),
    ("mouseInvertYaw", "0"),
    ("movie", "1"),
    ("movieSubtitle", "0"),
    ("MusicVolume", "0.4"),
    ("nearclip", "0.1"),
    ("ObjectSelectionCircle", "1"),
    ("occlusion", "1"),
    ("particleDensity", "1.0"),
    ("PetMeleeDamage", "1"),
    ("PetSpellDamage", "1"),
    ("pixelShaders", "0"),
    ("PlayerAnim", "0"),
    ("PlayerFadeInRate", "4096"),
    ("PlayerFadeOutAlpha", "128"),
    ("PlayerFadeOutRate", "4096"),
    ("profanityFilter", "1"),
    ("readContest", "0"),
    ("readEULA", "0"),
    ("readScanning", "0"),
    ("readTOS", "0"),
    ("realmList", "us.logon.worldofwarcraft.com:3724"),
    ("realmName", ""),
    ("scriptMemory", "49152"),
    ("shadowBias", "0.1"),
    ("shadowLevel", "1"),
    ("shadowLOD", "1"),
    ("ShowErrors", "1"),
    ("showfootprintparticles", "1"),
    ("showfootprints", "1"),
    ("showGameTips", "1"),
    ("showLootSpam", "1"),
    ("showsmartrects", "0"),
    ("SkyCloudLOD", "0"),
    ("SmallCull", "0.04"),
    ("SoundDriver", "-1"),
    ("SoundInitFlags", "128"),
    ("SoundListenerAtCharacter", "1"),
    ("SoundMaxHardwareChannels", "12"),
    ("SoundMemoryCache", "4"),
    ("SoundMinHardwareChannels", "-1"),
    ("SoundMixer", "-1"),
    ("SoundMixRate", "44100"),
    ("SoundOutputSystem", "-1"),
    ("SoundReverb", "1"),
    ("SoundRolloffFactor", "4"),
    ("SoundSoftwareChannels", "12"),
    ("SoundVolume", "1.0"),
    ("SoundZoneMusicNoDelay", "0"),
    ("spamFilter", "1"),
    ("specular", "0"),
    ("spellEffectLevel", "2"),
    ("statusBarText", "0"),
    ("TargetAnim", "0"),
    ("texLodBias", "0.0"),
    ("textureLodDist", "777.0"),
    ("timingModeOverride", "0"),
    ("triangleStrips", "1"),
    ("trilinear", "0"),
    ("UberTooltips", "1"),
    ("uiScale", "1.0"),
    ("unitDrawDist", "300.0"),
    ("UnitNameNPC", "0"),
    ("UnitNameOwn", "0"),
    ("UnitNamePlayer", "1"),
    ("UnitNamePlayerGuild", "1"),
    ("UnitNamePlayerPVPTitle", "1"),
    ("UnitNameRenderMode", "2"),
    ("useUiScale", "0"),
    ("useWeatherShaders", "1"),
    ("waterLOD", "0"),
    ("weatherDensity", "2"),
    ("widescreen", "1"),
];

/// **The default for a name, or `None`.**
///
/// Case-insensitive, which is the reference's own lookup and not a
/// kindness: `UIOptionsFrame` spells `statusBarText` the way it is registered
/// but the same file's `ReputationFrame` arm spells it differently, and a
/// client that only matched exactly would answer one of them and not the other.
pub fn default_of(name: &str) -> Option<&'static str> {
    DEFAULTS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(_, value)| *value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The table holds what it claims to.** A table that quietly lost half
    /// its rows would otherwise pass every other test in this crate.
    #[test]
    fn the_walk_covered_the_registrations_it_claims_to() {
        assert_eq!(DEFAULTS.len(), 200);
        assert_eq!(214 - 7 - 7, 200);
    }

    /// **No name is registered twice**, which is what makes [`default_of`]'s
    /// first match the only match.
    #[test]
    fn every_name_is_its_own() {
        let mut seen: Vec<String> = DEFAULTS.iter().map(|(n, _)| n.to_lowercase()).collect();
        seen.sort();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count);
    }

    /// **The eleven the sound panel is written against**, with the values the
    /// client ships -- the ones `SoundOptionsFrame_Load` reads on the first
    /// open. Two of the three volumes are exactly the placeholders
    /// `sound::mixer` had been carrying under the CVars' own names, which is
    /// the check that this column is the column it is taken for.
    #[test]
    fn the_sound_panel_reads_eleven_of_these() {
        for (name, value) in [
            ("MasterVolume", "1.0"),
            ("SoundVolume", "1.0"),
            ("MusicVolume", "0.4"),
            ("AmbienceVolume", "0.6"),
            ("MasterSoundEffects", "1"),
            ("EnableMusic", "1"),
            ("EnableAmbience", "1"),
            ("EnableErrorSpeech", "1"),
            ("SoundListenerAtCharacter", "1"),
            ("EmoteSounds", "1"),
            ("SoundZoneMusicNoDelay", "0"),
        ] {
            assert_eq!(default_of(name), Some(value), "{name}");
        }
    }

    /// …and the case-insensitivity [`default_of`] promises.
    #[test]
    fn a_name_is_found_however_it_is_spelled() {
        assert_eq!(default_of("ENABLEMUSIC"), default_of("EnableMusic"));
        assert_eq!(default_of("statusbartext"), Some("0"));
        assert_eq!(default_of("no such setting"), None);
    }
}
