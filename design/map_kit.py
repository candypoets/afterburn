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
from fc import F, H, Vt, T, R

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
DETAIL_W = 700


def ink(area):
    return AREAS.get(area, ACCENT)


def _t(text, size, font=SANS, style="Regular", color=FG, w=None, name=None, lh=None, ls=None,
       case=None, align="left", x=0, y=0):
    return T(text, size, font=font, style=style, color=color, w=w, name=name, lh=lh, ls=ls,
             case=case, align=align, x=x, y=y)


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


def detail_screen(detail):
    """A prototype screen for one area: heading, back link, and a slot for cards."""
    tint = ink(detail["area"])
    back = H(
        _t("←", 14, SANS, "Regular", tint, name="arrow"),
        _t("Back to the map", 11, MONO, "Medium", tint, name="label"),
        gap=6, cross="center", pad=[6, 10, 6, 10], fill="#F1EEE7", radius=4, name="Back to the map")
    cards = Vt(name="Cards", gap=10, w=DETAIL_W - 64)
    return Vt(
        _t(detail["eyebrow"], 10, MONO, "Medium", tint, name="eyebrow", ls=6),
        _t(detail["title"], 38, DISP, "Regular", FG, w=DETAIL_W - 64, name="title", lh=42),
        _t(detail["lede"], 14, SANS, "Regular", MUTED, w=DETAIL_W - 64, name="lede", lh=20),
        back,
        cards,
        gap=14, pad=32, w=DETAIL_W, fill=SURF, stroke=BORDER, radius=10, name="Detail screen")


def caption(text):
    return _t(text, 10, MONO, "Medium", MUTED, name="caption", ls=4)


def map_mode_screen(m):
    """A sketch of the human-facing surface. All coordinates are absolute."""
    w, h = 1080, 700
    tint = ink("COMPONENTS")

    def card(x, y, name, detail, selected=False):
        return F(
            "Card " + name,
            _t(name, 15, SANS, "Medium", FG, name="title", x=16, y=14),
            _t(detail, 12, SANS, "Regular", MUTED, w=348, name="detail", lh=17, x=16, y=40),
            x=x, y=y, w=380, h=116, fill=SURF,
            stroke=tint if selected else BORDER, sw=2 if selected else 1, radius=8, clip=True)

    changes = []
    y = 52
    for label, meta in m["changes"]:
        changes.append(_t(label, 12, SANS, "Medium", FG, name="change", x=16, y=y))
        changes.append(_t(meta, 9, MONO, "Regular", MUTED, name="meta", x=16, y=y + 17))
        y += 46

    return F(
        "Map mode",
        _t(m["breadcrumb"], 11, MONO, "Medium", FG, name="breadcrumb", x=28, y=24),
        _t(m["version"], 10, MONO, "Regular", MUTED, w=260, align="right", name="version", x=w - 316, y=25),
        _t(m["tag"], 9, MONO, "Medium", ACCENT, name="tag", ls=4, x=w - 316, y=52),
        _t("SELECTED", 9, MONO, "Medium", tint, name="selected tag", ls=4, x=32, y=76),
        card(32, 96, m["selected"], m["selected_detail"], selected=True),
        card(32, 240, m["other"], "Live thumbnails, search, drag onto the canvas.", selected=False),
        F("Changes",
          _t(m["changes_title"], 10, MONO, "Medium", MUTED, name="label", ls=6, x=16, y=16),
          *changes,
          x=760, y=96, w=288, h=470, fill=SURF, stroke=BORDER, radius=8, clip=True),
        F("Reply",
          _t("AGENT", 9, MONO, "Medium", ACCENT, name="who", ls=4, x=14, y=10),
          _t(m["reply"], 12, SANS, "Regular", FG, w=648, name="text", lh=17, x=14, y=26),
          x=32, y=470, w=680, h=58, fill="#F1EEE7", stroke=BORDER, radius=8, clip=True),
        F("Ask bar",
          _t(m["ask"], 13, SANS, "Regular", "#F2EFE8", w=620, name="question", x=16, y=14),
          _t("⌘↵", 11, MONO, "Regular", DISABLED, name="hint", x=636, y=15),
          x=32, y=600, w=680, h=48, fill=FG, radius=8, clip=True),
        w=w, h=h, fill=BG, clip=True, radius=12, stroke=BORDER)
