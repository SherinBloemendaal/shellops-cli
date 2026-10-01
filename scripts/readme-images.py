#!/usr/bin/env python3
"""Regenerate the README banner and how-it-works diagram in assets/ (light and dark files).

Edit the content in banner() / how_it_works() or the THEMES palette, then run: scripts/readme-images.py
"""

from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / 'assets'

SANS = "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Helvetica Neue', Helvetica, Arial, sans-serif"
MONO = "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace"

# A dark CRT terminal: phosphor green on black with scanlines. A terminal is dark either way,
# so the light and dark README variants share it (crepath uses warm orange + rose instead).
TERMINAL = dict(
    bg1='#010402', bg2='#04100A', dot='#22C55E', dot_op='0.06', border='#0F2A1A',
    text='#D1FAE0', muted='#6FA383', faint='#3F6B50',
    card='#020A05', card_border='#14532D', card_shadow='#000000', shadow_op='0.6',
    line='#0B2616', glow_op='0.14', code_bg='#03140A', code_text='#4ADE80',
    a1='#4ADE80', a2='#22C55E', a3='#166534', ok='#4ADE80', on_accent='#011006', warn='#FACC15',
)
THEMES = {'light': TERMINAL, 'dark': TERMINAL}


def esc(text: str) -> str:
    return text.replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;')


def defs(t: dict, w: int, h: int) -> str:
    return f'''<defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="{t['bg1']}"/><stop offset="1" stop-color="{t['bg2']}"/>
    </linearGradient>
    <linearGradient id="accent" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="{t['a1']}"/><stop offset="1" stop-color="{t['a2']}"/>
    </linearGradient>
    <linearGradient id="wire" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="{t['a3']}" stop-opacity="0.95"/><stop offset="1" stop-color="{t['a1']}" stop-opacity="0.9"/>
    </linearGradient>
    <radialGradient id="glow1"><stop offset="0" stop-color="{t['a1']}" stop-opacity="{t['glow_op']}"/><stop offset="1" stop-color="{t['a1']}" stop-opacity="0"/></radialGradient>
    <radialGradient id="glow2"><stop offset="0" stop-color="{t['a3']}" stop-opacity="{t['glow_op']}"/><stop offset="1" stop-color="{t['a3']}" stop-opacity="0"/></radialGradient>
    <pattern id="dots" width="22" height="22" patternUnits="userSpaceOnUse">
      <circle cx="2" cy="2" r="1.2" fill="{t['dot']}" fill-opacity="{t['dot_op']}"/>
    </pattern>
    <pattern id="scan" width="4" height="3" patternUnits="userSpaceOnUse">
      <rect width="4" height="1" fill="#000000" fill-opacity="0.35"/>
    </pattern>
    <filter id="phosphor" x="-10%" y="-40%" width="120%" height="180%">
      <feGaussianBlur stdDeviation="3.2" result="blur"/>
      <feMerge><feMergeNode in="blur"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
    <clipPath id="frame"><rect width="{w}" height="{h}" rx="14"/></clipPath>
  </defs>'''


def backdrop(t: dict, w: int, h: int, glows: list[tuple[int, int, int, str]]) -> str:
    glow = ''.join(f'<circle cx="{x}" cy="{y}" r="{r}" fill="url(#{g})"/>' for x, y, r, g in glows)
    return f'''<g clip-path="url(#frame)">
    <rect width="{w}" height="{h}" fill="url(#bg)"/>
    <rect width="{w}" height="{h}" fill="url(#dots)"/>
    {glow}
  </g>
  <rect x="0.75" y="0.75" width="{w - 1.5}" height="{h - 1.5}" rx="23.25" fill="none" stroke="{t['border']}" stroke-width="1.5"/>'''.replace('rx="23.25"', 'rx="13.25"')


def card(t: dict, x: float, y: float, w: float, h: float, rx: int = 6, opacity: float = 1) -> str:
    return (
        f'<rect x="{x}" y="{y + 5}" width="{w}" height="{h}" rx="{rx}" fill="{t["card_shadow"]}" fill-opacity="{t["shadow_op"]}"/>'
        f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{t["card"]}" fill-opacity="{opacity}" stroke="{t["card_border"]}" stroke-width="1.25"/>'
    )


def badge(x: float, y: float, label: str, color: str, size: int = 34) -> str:
    font = 12.5 if len(label) < 3 else 10.5
    return (
        f'<rect x="{x - size / 2}" y="{y - size / 2}" width="{size}" height="{size}" rx="4" fill="{color}" fill-opacity="0.08" '
        f'stroke="{color}" stroke-width="1.5"/>'
        f'<text x="{x}" y="{y + 4.3}" text-anchor="middle" font-family="{MONO}" font-size="{font}" font-weight="800" fill="{color}">{label}</text>'
    )


def text(x: float, y: float, value: str, size: float, fill: str, *, mono: bool = False, weight: int = 400,
         anchor: str = 'start', spacing: float = 0, opacity: float = 1) -> str:
    extra = f' letter-spacing="{spacing}"' if spacing else ''
    extra += f' text-anchor="{anchor}"' if anchor != 'start' else ''
    extra += f' fill-opacity="{opacity}"' if opacity != 1 else ''
    return (f'<text x="{x}" y="{y}" font-family="{MONO if mono else SANS}" font-size="{size}" '
            f'font-weight="{weight}" fill="{fill}"{extra}>{esc(value)}</text>')


def wire(x1: float, y1: float, x2: float, y2: float, bend: float = 50, width: float = 2) -> str:
    if abs(y2 - y1) < 0.5:
        y2 = y1 + 0.5  # a perfectly flat path has a zero-height box, which hides a gradient stroke
    return (f'<path d="M{x1} {y1} C {x1 + bend} {y1}, {x2 - bend} {y2}, {x2} {y2}" fill="none" '
            f'stroke="url(#wire)" stroke-width="{width}" stroke-linecap="round"/>')


def hero(t: dict, eyebrow: str, plain: str, accent: str, tagline: str, command: str) -> list[str]:
    pill_w = len(command) * 9.45 + 70
    return [
        text(72, 104, eyebrow, 14, t['muted'], mono=True, weight=600, spacing=1.5),
        f'<text x="68" y="184" font-family="{MONO}" font-size="76" font-weight="800" letter-spacing="-3" '
        f'fill="{t["text"]}" filter="url(#phosphor)">{esc(plain)}<tspan fill="url(#accent)">{esc(accent)}</tspan></text>',
        text(72, 232, tagline, 19, t['muted'], mono=True),
        f'<rect x="72" y="264" width="{pill_w}" height="42" rx="6" fill="{t["code_bg"]}" stroke="{t["card_border"]}"/>',
        text(110, 290.5, command, 15.5, t['code_text'], mono=True, weight=600),
        f'<rect x="{72 + pill_w - 26}" y="{276}" width="9" height="18" fill="{t["a1"]}" filter="url(#phosphor)"/>',
    ]


def column_label(x: float, value: str, color: str) -> str:
    return text(x, 58, f'> {value.lower()}', 14, color, mono=True, weight=700, spacing=0.5)


def svg(w: int, h: int, title: str, parts: list[str]) -> str:
    body = '\n  '.join(parts)
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" role="img" aria-label="{esc(title)}">
  <title>{esc(title)}</title>
  {body}
  <rect width="{w}" height="{h}" fill="url(#scan)" clip-path="url(#frame)" pointer-events="none"/>
</svg>
'''


def flow(t: dict, title: str, labels: tuple[str, str, str], sources: list[tuple[str, list[str]]],
         center: tuple[str, list[str]], rows: list[tuple[str, str, str, str]], *, row_mono: bool = False) -> str:
    """Three columns: source cards, one center box, and a list of destination rows."""
    w = 1280
    th, gap, top = 66, 12, 84
    h = max(560, top + len(rows) * (th + gap) + 40)
    mid = (top + top + len(rows) * (th + gap) - gap) / 2
    parts = [defs(t, w, h), backdrop(t, w, h, [(230, mid, 280, 'glow1'), (640, mid, 220, 'glow2'), (1050, mid, 320, 'glow1')])]
    parts += [column_label(x, label, t['a1']) for x, label in zip((48, 488, 800), labels)]

    sx, sw = 48, 340
    heights = [70 + len(lines) * 28 for _, lines in sources]
    total = sum(heights) + 40 * (len(sources) - 1)
    y = mid - total / 2
    ix, iw = 488, 260
    ih = 72 + len(center[1]) * 26
    iy = mid - ih / 2
    for (name, lines), sh in zip(sources, heights):
        parts.append(wire(sx + sw, y + sh / 2, ix, mid, bend=60, width=2.25))
        parts.append(card(t, sx, y, sw, sh))
        parts.append(f'<rect x="{sx}" y="{y + 18}" width="5" height="{sh - 36}" rx="2.5" fill="url(#accent)"/>')
        parts.append(text(sx + 28, y + 42, name, 17, t['text'], mono=True, weight=700))
        for i, line in enumerate(lines):
            parts.append(text(sx + 28, y + 76 + i * 28, '▸', 14, t['a1'], mono=True))
            parts.append(text(sx + 46, y + 76 + i * 28, line, 14, t['muted'], mono=True))
        y += sh + 40

    parts.append(card(t, ix, iy, iw, ih, rx=6))
    parts.append(f'<rect x="{ix}" y="{iy}" width="{iw}" height="44" rx="6" fill="url(#accent)"/>')
    parts.append(f'<rect x="{ix}" y="{iy + 26}" width="{iw}" height="18" fill="url(#accent)"/>')
    parts.append(text(ix + iw / 2, iy + 28, center[0], 15.5, t['on_accent'], mono=True, weight=700, anchor='middle'))
    for i, line in enumerate(center[1]):
        parts.append(text(ix + 20, iy + 76 + i * 26, '✓', 14, t['ok'], mono=True, weight=700))
        parts.append(text(ix + 40, iy + 76 + i * 26, line, 13.5, t['muted'], mono=True))

    tx, tw = 800, 432
    for i, (label, color, name, detail) in enumerate(rows):
        ty = top + i * (th + gap)
        cy = ty + th / 2
        parts.append(wire(ix + iw, mid, tx, cy, bend=40))
        parts.append(f'<circle cx="{tx}" cy="{cy}" r="3.5" fill="{t["a1"]}"/>')
        parts.append(card(t, tx, ty, tw, th))
        parts.append(badge(tx + 34, cy, label, color))
        parts.append(text(tx + 66, ty + 28, name, 15.5, t['text'], mono=True, weight=700))
        parts.append(text(tx + 66, ty + 50, detail, 12.5, t['muted'], mono=True))
    parts.append(f'<circle cx="{ix + iw}" cy="{mid}" r="5" fill="{t["a1"]}" stroke="{t["card"]}" stroke-width="2"/>')

    return svg(w, h, title, parts)


def write_all(banner, how_it_works) -> None:
    OUT.mkdir(exist_ok=True)
    for theme, colors in THEMES.items():
        (OUT / f'banner-{theme}.svg').write_text(banner(colors))
        (OUT / f'how-it-works-{theme}.svg').write_text(how_it_works(colors))
    print(sorted(p.name for p in OUT.glob('*.svg')))


def banner(t: dict) -> str:
    w, h = 1280, 360
    parts = [defs(t, w, h), backdrop(t, w, h, [(1000, 180, 320, 'glow1'), (1220, 340, 220, 'glow2'), (110, 10, 220, 'glow2')])]
    parts += hero(t, '# docker compose · php tooling · releases', 'Shell', 'Ops',
                  'DevOps for your Compose + PHP project, from the shell.', '$ so cc')

    # Terminal window
    x, y, tw, th = 760, 52, 460, 256
    parts.append(card(t, x, y, tw, th, rx=6))
    parts.append(f'<path d="M{x} {y + 6} a6 6 0 0 1 6 -6 h{tw - 12} a6 6 0 0 1 6 6 v30 h{-tw} z" fill="{t["line"]}"/>')
    for i, color in enumerate(['#FF5F57', '#FEBC2E', '#28C840']):
        parts.append(f'<circle cx="{x + 22 + i * 18}" cy="{y + 18}" r="5.5" fill="{color}"/>')
    parts.append(text(x + tw / 2, y + 23, '~/projects/api', 12.5, t['faint'], mono=True, anchor='middle'))

    lines = [
        ('$', 'so cc', t['a1'], t['text']),
        ('✓', 'filesystem + Symfony caches', t['ok'], t['muted']),
        ('✓', 'redis', t['ok'], t['muted']),
        ('✓', 'php reload + messenger workers', t['ok'], t['muted']),
        ('$', 'so release', t['a1'], t['text']),
        ('?', 'bump  patch · minor · major', t['warn'], t['muted']),
    ]
    for i, (mark, value, mark_color, color) in enumerate(lines):
        ly = y + 70 + i * 28
        parts.append(text(x + 26, ly, mark, 15, mark_color, mono=True, weight=700))
        parts.append(text(x + 48, ly, value, 15, color, mono=True, weight=600 if mark == '$' else 400))
    cy = y + 70 + len(lines) * 28
    parts.append(text(x + 26, cy, '$', 15, t['a1'], mono=True, weight=700))
    parts.append(f'<rect x="{x + 48}" y="{cy - 13}" width="9" height="17" fill="{t["a1"]}" filter="url(#phosphor)"/>')

    return svg(w, h, 'ShellOps: DevOps for your Compose + PHP project, from the shell', parts)


def how_it_works(t: dict) -> str:
    return flow(
        t,
        'How so works: it reads the project at the git root and runs Docker, PHP tooling, database, cache, release and alias commands',
        ('YOUR PROJECT', 'ONE ENTRY POINT', 'WHAT IT RUNS'),
        [
            ('<git root>/', ['compose.yml', '.env  (APP_ENV=dev or prod)', '*.Dockerfile', '.shellops/aliases/']),
            ('~/.shellops/', ['config.toml', 'secrets.toml  (0600)', 'trust.toml']),
        ],
        ('so <command>', ['finds the git root', 'loads .env: dev or prod', 'runs in the right service']),
        [
            ('DC', '#4ADE80', 'Docker', 'compose · build · installer'),
            ('PHP', '#4ADE80', 'PHP tooling', 'console · composer · phpstan · phpunit'),
            ('DB', '#4ADE80', 'Database', 'fixtures · reset-database · fdd'),
            ('CC', '#4ADE80', 'Caches', 'cc: files, Redis, PHP + workers'),
            ('REL', '#4ADE80', 'Releases', 'bump · tag · push · GitHub notes'),
            ('AL', '#4ADE80', 'Aliases', 'SSH-signed, trusted per body or author'),
        ],
    )


if __name__ == '__main__':
    write_all(banner, how_it_works)
