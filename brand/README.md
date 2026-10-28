# Sofa Remote brand

**Idea:** a sofa whose backrest is a phone screen. Comfort plus control, in one shape.
**Voice:** relaxed, direct, a little playful. "Run your PC from the sofa." Short sentences, no jargon
up front; the technical detail (UDP, bytes, latency) comes as proof further down.

## Logo

| File | Use |
| --- | --- |
| `logo.png` | Navy logo, transparent, for light backgrounds |
| `logo-on-dark.png` | Off-white logo, transparent, for dark backgrounds (the default: the brand lives in the dark) |
| `logo-256.png`, `logo-on-dark-256.png` | Small versions for READMEs and web |
| `icon-512.png` | App-style icon: dark logo on an ink rounded square |
| `concepts/` | The original generated images (and their prompts in the `.json` sidecars), plus the hero art |

Keep clear space around the mark of about half its height. Don't recolour the screen or the
cushions, don't add outlines or shadows, and don't put it on the blue (the screen disappears).

## Colours

| Name | Hex | Use |
| --- | --- | --- |
| Ink | `#0E0F13` | Backgrounds |
| Night | `#151A2E` | Logo body on light, deep panels |
| Card | `#171A24` | Cards and surfaces on Ink |
| Screen blue | `#4F8CFF` | Primary action, links, the phone screen |
| Light blue | `#7AA7FF` | Accent text on dark |
| Cushion amber | `#FFB547` | Highlights, labels, small accents only |
| Paper | `#E8E9EE` | Text on dark, logo body on dark |

## Type

- Headings: **Bricolage Grotesque** 600/800, tight tracking (-0.02em)
- Body: **DM Sans** 400/500/700

Both are free on Google Fonts. The phone app itself stays on the system font for speed.

## Regenerating

The logo and hero art were made with `gpt-image-2`. The prompts are in `concepts/*.json`.
The icon sizes, site images and the Android launcher foreground are rendered from `logo.png` and
`logo-on-dark.png`.
