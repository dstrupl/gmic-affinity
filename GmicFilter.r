// GmicFilter.r — legacy compiled pipl resource source.
//
// Why this file is required even though we ship PiPLs.json:
//   Affinity Photo 2 does NOT read Adobe's modern PiPLs.json. It only
//   recognises plugins that carry a binary `PiPL` resource at
//   Contents/Resources/<CFBundleExecutable>.rsrc. With no .rsrc Affinity
//   silently rejects the bundle during enumeration (see the fs_usage
//   analysis recorded in the repo history) — it never reads the Mach-O
//   and never appears in the Detected Plugins list. AKVIS, Topaz, Nik,
//   Filter Forge etc. all ship this legacy .rsrc; that is exactly why
//   they "just work" in Affinity.
//
// The Rez compiler (`/usr/bin/Rez`, still shipped with Xcode CLT) turns
// this file into a binary PiPL resource that we then place in the bundle
// as Contents/Resources/GmicFilter.rsrc.

#include "PIDefines.h"
#include "PIGeneral.r"
// NB: we deliberately do NOT #include "PIUtilities.r" because it
// references the Carbon `Types.r` / `SysTypes.r` headers (for the
// classic 'STR ' and 'vers' resource types) which Apple removed from
// the CommandLineTools SDKs years ago. We don't need either resource
// for plugin discovery, so excluding the file lets Rez succeed on a
// stock Xcode-CLT install.

resource 'PiPL' (16000, "GmicFilter PiPL", purgeable)
{
    {
        // Plug-in kind. 'Filter' = '8BFM', same FourCC as
        // CFBundlePackageType in Info.plist.
        Kind { Filter },

        // Menu label. Trailing "..." follows the Photoshop convention
        // for filters that pop a dialog (we'll have one in M5+).
        Name { "G'MIC..." },

        // Submenu under Filters > Plugins. Affinity uses this as the
        // category name; AKVIS / Nik etc. use their company name here.
        Category { "G'MIC" },

        // Version of the *PiPL data*, not of our plugin. Adobe SDK
        // defines latestFilterVersion / latestFilterSubVersion in
        // PIFilter.h / PIGeneral.r.
        Version { (latestFilterVersion << 16) | latestFilterSubVersion },

        // Entry-point function names per slice. The host looks these
        // symbols up via dlsym() on the Mach-O. Matches the
        // `#[no_mangle] pub unsafe extern "C" fn PluginMain` exported
        // by src/lib.rs.
        CodeMacIntel64 { "PluginMain" },
        CodeMacARM64   { "PluginMain" },

        // Image modes the filter is willing to be offered for. Mirrors
        // PiPLs.json's "SupportedModes". We restrict to 8-bit Grayscale
        // and RGB(A) for v1 because the pixel pipeline assumes those
        // (see src/filter.rs and PRD §5).
        SupportedModes
        {
            noBitmap,            doesSupportGrayScale,
            noIndexedColor,      doesSupportRGBColor,
            noCMYKColor,         noHSLColor,
            noHSBColor,          noMultichannel,
            noDuotone,           noLABColor
        },

        // Scriptable image-mode predicate. The host evaluates this and
        // greys out the menu entry when the active document doesn't
        // match. Limiting to 8-bit RGB / Gray for v1 mirrors filter.rs.
        EnableInfo { "in (PSHOP_ImageMode, RGBMode, GrayScaleMode)" },

        // Cap on the largest image we ask the host to hand us (pixels).
        // Same value the dissolve sample ships; keeps the host from
        // tiling huge images for us until we implement tiling support.
        PlugInMaxSize { 2000000, 2000000 },

        // Retina / HiDPI awareness. We don't open any UI yet (just a
        // dialog-less filter), but advertising this avoids the host
        // running us through its NSScreen up-scaler. PIPL.r treats
        // the *presence* of this property as the on-signal, so no
        // value is supplied (see PIPL.r: case MonitorScalingAware).
        MonitorScalingAware {},
    }
};
