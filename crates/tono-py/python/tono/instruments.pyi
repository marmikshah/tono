"""The catalog instrument constructors — each returns a `tono.Voice`.

Variant names mirror the `tono catalog` CLI slugs; an unknown variant raises
`ValueError` listing the valid ones.
"""

from tono import Voice

def piano(variant: str = "grand") -> Voice:
    """Variants: grand (default), bright, mellow, felt, upright, honky-tonk."""
    ...
def electric_piano(variant: str = "rhodes") -> Voice:
    """Variants: rhodes (default), wurli, dx."""
    ...
def organ(variant: str = "tonewheel") -> Voice:
    """Variants: tonewheel (default), rock."""
    ...
def strings(variant: str = "warm") -> Voice:
    """Variants: warm (default), ensemble."""
    ...
def bass(variant: str = "finger") -> Voice:
    """Variants: finger (default), pick, sub, synth."""
    ...
def guitar(variant: str = "nylon") -> Voice:
    """Variants: nylon (default), steel, electric."""
    ...
def drums(variant: str = "acoustic") -> Voice:
    """Variants: acoustic (default), classic, electronic, tr808."""
    ...
def brass(variant: str = "section") -> Voice:
    """Variants: section (default), stab."""
    ...
def flute(variant: str = "concert") -> Voice:
    """Variants: concert (default)."""
    ...
def mallets(variant: str = "marimba") -> Voice:
    """Variants: marimba (default), vibraphone, glockenspiel."""
    ...
def bells(variant: str = "tubular") -> Voice:
    """Variants: tubular (default)."""
    ...
