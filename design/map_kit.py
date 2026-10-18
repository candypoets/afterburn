"""The component kit for "FigCraft · feature map".

Mains are drawn with fc.py on the Components page and turned into FigCraft
components by the build; every card on every page is an instance, so changing a
main changes the whole map.

  Branch    variant by Area; text properties Name, Role, Source; slot Code
            (its leaves) — an area of the tool
  Leaf      variant by Area; text properties Title, Fact, Source — one capability
  Stage     one step of a command's path: Number, Name, Detail, Source
  Surfaces  the "same command, every surface" card: Body, Source
  Root      the map's root card: Claim, Facts, Source
"""
from fc import H, Vt, T, R

BG = "#E9E7E2"
SURF = "#FFFFFF"
CARD = "#F5F2EC"
FG = "#1B150D"
MUTED = "#625D56"
BORDER = "#D6D2C9"
ACCENT = "#B5452F"
DISABLED = "#8C867C"

DISP = "Charter"
SANS = "Inter"
MONO = "JetBrains Mono"

# Feature area → ink. One ink per area, on the badge and the leaf rail.
AREAS = {
    "CANVAS": "#8A6D2B",
    "LAYOUT": "#2F6F6A",
    "COMPONENTS": "#6B4E8C",
    "STYLES": "#55602A",
    "VECTOR": "#3E5A78",
    "PAINT": "#B5452F",
    "TEXT": "#1B150D",
    "PROTOTYPE": "#7A4A6B",
    "HANDOFF": "#255D34",
    "FILES": "#625D56",
    "MULTIPLAYER": "#2F5D7A",
    "EXTEND": "#8C2F1F",
}

BRANCH_W = 700
LEAF_W = 620
ROOT_W = 420
STAGE_W = 300
SURFACES_W = 1180


def ink(area):
    return AREAS.get(area, ACCENT)


def _t(text, size, font=SANS, style="Regular", color=FG, w=None, name=None, lh=None, ls=None, case=None):
    return T(text, size, font=font, style=style, color=color, w=w, name=name, lh=lh, ls=ls, case=case)


def heading(title, lede, w=2740):
    return Vt(
        _t(title, 52, DISP, "Regular", FG, w=w, lh=58, name="title"),
        _t(lede, 17, SANS, "Regular", MUTED, w=min(w, 1080), lh=26, name="lede"),
        gap=12, w=w, name="heading")


def root_main():
    return Vt(
        _t("FEATURE MAP", 10, MONO, "Medium", ACCENT, name="eyebrow", ls=6),
        _t("FigCraft", 46, DISP, "Regular", FG, w=ROOT_W - 52, name="name", lh=50),
        _t("A Figma-class design tool in pure Rust, where every feature is a command.",
           15, SANS, "Regular", MUTED, w=ROOT_W - 52, name="claim", lh=22),
        _t("16 crates · 4 apps", 11, MONO, "Medium", FG, w=ROOT_W - 52, name="facts", lh=18),
        _t("README.md:0", 10, MONO, "Regular", "#7B7469", name="source"),
        gap=10, pad=26, w=ROOT_W, fill=SURF, stroke=BORDER, radius=12, name="Root")


def branch_main(area):
    tint = ink(area)
    return Vt(
        H(R(10, 10, fill=tint, radius=5, name="dot"),
          _t(area, 10, MONO, "Medium", tint, name="area", ls=6),
          gap=8, cross="center", name="head"),
        _t("Area", 27, DISP, "Regular", FG, w=BRANCH_W - 40, name="title", lh=32),
        _t("What this area is for", 14, SANS, "Regular", MUTED, w=BRANCH_W - 40, name="role", lh=20),
        _t("path:0", 10.5, MONO, "Regular", "#7B7469", name="source"),
        Vt(name="Code", gap=8, w=BRANCH_W - 40),
        gap=12, pad=[18, 20, 20, 20], w=BRANCH_W, fill=SURF, stroke=BORDER, radius=12,
        name=f"Area={area}")


def leaf_main(area):
    tint = ink(area)
    return Vt(
        _t("Leaf", 15, SANS, "Medium", FG, w=LEAF_W - 32, name="title", lh=20),
        _t("What you can do, and how it is built", 13, SANS, "Regular", MUTED, w=LEAF_W - 32,
           name="fact", lh=18),
        _t("path:0", 10.5, MONO, "Regular", "#7B7469", name="source"),
        gap=4, pad=[11, 12, 11, 14], w=LEAF_W, fill=CARD, stroke=tint, sw=3,
        strokeSides=[0, 0, 0, 1], radius=0, name=f"Area={area}")


def stage_main():
    return Vt(
        R(STAGE_W, 4, fill=FG, name="rail"),
        Vt(
            _t("01", 10, MONO, "Medium", ACCENT, name="number", ls=6),
            _t("Stage", 22, DISP, "Regular", FG, w=STAGE_W - 32, name="name", lh=26),
            _t("What happens here", 13, SANS, "Regular", MUTED, w=STAGE_W - 32, name="detail", lh=18),
            _t("path:0", 10.5, MONO, "Regular", "#7B7469", name="source"),
            gap=8, pad=[13, 15, 15, 15], w=STAGE_W, name="body"),
        w=STAGE_W, fill=SURF, stroke=BORDER, radius=6, clip=True, name="Stage")


def surfaces_main():
    return Vt(
        _t("THE SEAM", 10, MONO, "Medium", ACCENT, name="kind", ls=8),
        _t("Same command, every surface", 28, DISP, "Regular", FG, w=SURFACES_W - 64,
           name="title", lh=32),
        _t("The menu, a shortcut, the Actions palette, the CLI, the MCP server, the control "
           "channel and a plugin all dispatch the same command ids.", 14, SANS, "Regular", FG,
           w=SURFACES_W - 64, name="body", lh=20),
        _t("MENU   SHORTCUT   ACTIONS   CLI   MCP   CONTROL   PLUGIN", 11, MONO, "Medium", MUTED,
           w=SURFACES_W - 64, name="chips", lh=18),
        _t("path:0", 10.5, MONO, "Regular", "#7B7469", name="source"),
        gap=10, pad=26, w=SURFACES_W, fill=SURF, stroke=BORDER, radius=12, name="Surfaces")


def key_card():
    rows = []
    for label, text in [
        ("AREA", "one part of the feature surface, one ink"),
        ("LEAF", "a capability and the shape of its implementation"),
        ("SLOT", "leaves live inside their area — containment is the model"),
        ("SOURCE", "path:line, resolved from the repo at build time"),
    ]:
        rows.append(Vt(_t(label, 10, MONO, "Medium", MUTED, name="label", ls=6),
                       _t(text, 12, SANS, "Regular", FG, w=ROOT_W - 84, name="text", lh=17),
                       gap=2, name=label))
    return Vt(
        _t("How to read this map", 20, DISP, "Regular", FG, w=ROOT_W - 52, name="title", lh=24),
        _t("A branch is an area of the tool. Its leaves are the things you can do there, and "
           "the line under each one is the code that makes it work.", 13, SANS, "Regular",
           MUTED, w=ROOT_W - 52, name="lede", lh=19),
        *rows,
        gap=14, pad=24, w=ROOT_W, fill=SURF, stroke=BORDER, radius=12, name="Reading key")


def stats_card(lines):
    return Vt(
        _t("THE MAP, MEASURED", 10, MONO, "Medium", MUTED, name="label", ls=8),
        *[_t(line, 12, MONO, "Regular", FG, w=ROOT_W - 52, name="line", lh=19) for line in lines],
        gap=7, pad=22, w=ROOT_W, fill=SURF, stroke=BORDER, radius=12, name="Stats")


def arrow():
    return Vt(_t("→", 20, SANS, "Regular", DISABLED, name="glyph"),
              gap=2, w=44, h=40, cross="center", main="center", name="Arrow")


def find(node, name):
    if node.name == name:
        return node
    for child in node.children:
        found = find(child, name)
        if found:
            return found
    return None
