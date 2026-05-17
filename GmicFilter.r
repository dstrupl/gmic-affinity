/* GmicFilter.r
 *
 * Rez source for the `pipl` resource that tells Affinity Photo
 * (and any Photoshop-compatible host) the menu name, category, and
 * supported image modes for this filter plugin.
 *
 * Build:
 *   make pipl      # produces GmicFilter.rsrc
 * which expands to:
 *   Rez -o GmicFilter.rsrc GmicFilter.r \
 *       -i $PHOTOSHOP_SDK/photoshopapi/resources
 *
 * Required headers (shipped in the Adobe Photoshop SDK):
 *   - PIDefines.h
 *   - PITypes.r
 *   - PIGeneral.r
 *
 * See SETUP.md for SDK acquisition instructions.
 */

#include "PIDefines.h"
#include "PITypes.r"
#include "PIGeneral.r"

#ifndef ResourceID
#define ResourceID  16000
#endif

resource 'pipl' (ResourceID, "G'MIC", purgeable)
{
    {
        Kind { Filter },
        Name { "G'MIC..." },
        Category { "G'MIC" },
        Version { (latestFilterVersion << 16) | latestFilterSubVersion },

        /* v1 supports 8-bit RGB / RGBA only.
         * Other modes are explicitly excluded so the host doesn't even offer
         * the menu entry for them. Update this list as we add support. */
        SupportedModes {
            noBitmap,        noGrayScale,    noIndexedColor,
            doesRGB,         noCMYK,         noHSL,
            noHSB,           noMultichannel, noDuotone,
            noLab,           noGray16,       noRGB48,
            noLab48,         noCMYK64,       noDeepMultichannel,
            noDuotone16
        },

        HasTerminology
        {
            plugInClassFilter,
            plugInEventFilter,
            ResourceID,
            ""  /* unique scripting suite ID; empty = derive from plugin id */
        },

        /* No optional UI for v1 (no PARM resource, no callback). */
        EnableInfo { "in (PSHOP_ImageMode, RGBMode, RGBColorMode)" }
    }
};
