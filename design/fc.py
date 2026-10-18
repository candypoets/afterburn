"""Tiny builder that writes .figcraft documents with auto layout.

Usage: build trees with F() (frame), T() (text), R() (rectangle), E() (ellipse),
V() (vector); add them to pages; save(). Frames store gap+0.5 so that a
subsequent relayout pass (node.set gap=<real>) forces FigCraft to lay out.
"""
import json
import zipfile


def rgba(hex_):
    h = hex_.lstrip("#")
    a = int(h[6:8], 16) / 255 if len(h) == 8 else 1.0
    return {"r": int(h[0:2], 16) / 255, "g": int(h[2:4], 16) / 255, "b": int(h[4:6], 16) / 255, "a": a}


def paint(c):
    if isinstance(c, dict):
        return c
    return {"type": "solid", "color": rgba(c), "opacity": 1.0, "visible": True, "blend": "normal"}


class Node:
    def __init__(self, type_, name, **kw):
        self.type = type_
        self.name = name
        self.kw = kw
        self.children = []
        self.id = None

    def add(self, *kids):
        for k in kids:
            if k is None:
                continue
            if isinstance(k, (list, tuple)):
                self.add(*k)
            else:
                self.children.append(k)
        return self


def F(name="frame", *kids, dir=None, gap=0, pad=0, w=None, h=None, fill=None, stroke=None,
      sw=1, radius=0, main="min", cross="min", sh=None, sv=None, wrap=False, clip=False,
      effects=None, x=0, y=0, strokeSides=None, opacity=None, absolute=False, cgap=None):
    n = Node("frame", name, dir=dir, gap=gap, pad=pad, w=w, h=h, fill=fill, stroke=stroke, sw=sw,
             radius=radius, main=main, cross=cross, sh=sh, sv=sv, wrap=wrap, clip=clip,
             effects=effects, x=x, y=y, strokeSides=strokeSides, opacity=opacity,
             absolute=absolute, cgap=cgap)
    n.add(*kids)
    return n


def H(*kids, **kw):
    kw.setdefault("dir", "horizontal")
    name = kw.pop("name", "row")
    return F(name, *kids, **kw)


def Vt(*kids, **kw):
    kw.setdefault("dir", "vertical")
    name = kw.pop("name", "col")
    return F(name, *kids, **kw)


def T(text, size=14, font="Inter", style="Regular", color="#000000", w=None, sh=None,
      lh=None, ls=None, align="left", name=None, case=None, tnum=False, opacity=None,
      x=0, y=0):
    return Node("text", name or text[:40], text=text, size=size, font=font, style=style,
                color=color, w=w, sh=sh, lh=lh, ls=ls, align=align, case=case, tnum=tnum,
                opacity=opacity, x=x, y=y)


def R(w, h, fill=None, radius=0, name="rect", sh=None, sv=None, stroke=None, sw=1):
    return Node("rectangle", name, w=w, h=h, fill=fill, radius=radius, sh=sh, sv=sv,
                stroke=stroke, sw=sw)


def E(w, h, fill=None, name="dot", stroke=None, sw=1):
    return Node("ellipse", name, w=w, h=h, fill=fill, stroke=stroke, sw=sw)


def V(path, w, h, stroke=None, sw=1.5, fill=None, name="icon"):
    return Node("vector", name, path=path, w=w, h=h, stroke=stroke, sw=sw, fill=fill)


def _stroke(c, w, sides=None, align="inside"):
    return [paint(c)], {"weight": float(w), "align": align, "sides": [float(v) for v in sides] if sides else None,
                        "cap_start": "none", "cap_end": "none", "join": "miter", "miter_limit": 4.0, "dashes": []}


def shadow(y=2, blur=8, color="#0000001A", spread=0):
    return {"type": "dropShadow", "color": rgba(color), "offset": [0.0, float(y)], "radius": float(blur),
            "spread": float(spread), "blend": "normal", "show_behind": False, "visible": True}


class Doc:
    def __init__(self, name):
        self.name = name
        self.nodes = {}
        self.pages = []
        self.next = 1
        self.layout_frames = []  # (depth, id, gap)

    def _id(self):
        i = self.next
        self.next += 1
        return i

    def page(self, name, bg="#EDEBE6"):
        pid = self._id()
        self.nodes[str(pid)] = {"id": pid, "name": name, "type": "page", "background": rgba(bg),
                                "flows": [], "guides": [], "children": [], "width": 0.0, "height": 0.0}
        self.pages.append(pid)
        return pid

    def place(self, pid, node, x, y):
        node.kw["x"], node.kw["y"] = x, y
        self._emit(node, pid, 0, top=True)

    def _emit(self, n, parent, depth, top=False, parent_dir=None):
        nid = self._id()
        n.id = nid
        kw = n.kw
        d = {"id": nid, "name": n.name, "type": n.type, "parent": parent, "blend": "normal"}
        x, y = kw.get("x", 0) or 0, kw.get("y", 0) or 0
        d["transform"] = [1.0, 0.0, 0.0, 1.0, float(x), float(y)]
        cl = {"sizing_h": "fixed", "sizing_v": "fixed", "absolute": bool(kw.get("absolute")),
              "min_w": None, "max_w": None, "min_h": None, "max_h": None,
              "grid_column": None, "grid_row": None, "grid_column_span": 0, "grid_row_span": 0}
        if n.type == "frame":
            dir_ = kw["dir"]
            d["width"] = float(kw["w"] or 10)
            d["height"] = float(kw["h"] or 10)
            d["fills"] = [paint(kw["fill"])] if kw["fill"] else []
            if kw["stroke"]:
                d["strokes"], d["stroke"] = _stroke(kw["stroke"], kw["sw"], kw["strokeSides"])
            r = kw["radius"]
            if r:
                d["corners"] = {"radii": [float(v) for v in (r if isinstance(r, list) else [r] * 4)], "smoothing": 0.6}
            if kw["effects"]:
                d["effects"] = kw["effects"]
            d["clip"] = bool(kw["clip"])
            if dir_:
                p = kw["pad"]
                p = p if isinstance(p, list) else [p] * 4
                if len(p) == 2:
                    p = [p[0], p[1], p[0], p[1]]
                d["layout"] = {"mode": dir_, "wrap": kw["wrap"], "gap": float(kw["gap"]) + 0.5,
                               "cross_gap": float(kw["cgap"] if kw["cgap"] is not None else kw["gap"]),
                               "padding": [float(v) for v in p], "main_align": kw["main"],
                               "cross_align": kw["cross"], "grid_columns": 2, "grid_rows": 2,
                               "column_tracks": [], "row_tracks": [], "strokes_included": True,
                               "reverse_z": False}
                self.layout_frames.append((depth, nid, float(kw["gap"])))
            cl["sizing_h"] = kw["sh"] or ("hug" if (dir_ and kw["w"] is None) else "fixed")
            cl["sizing_v"] = kw["sv"] or ("hug" if (dir_ and kw["h"] is None) else "fixed")
            if kw["opacity"] is not None:
                d["opacity"] = kw["opacity"]
        elif n.type == "text":
            d["characters"] = kw["text"]
            st = {"font_family": kw["font"], "font_style": kw["style"], "font_size": float(kw["size"]),
                  "line_height": {"unit": "auto"} if kw["lh"] is None else {"unit": "pixels", "value": float(kw["lh"])},
                  "letter_spacing": {"unit": "percent", "value": float(kw["ls"] or 0)},
                  "decoration": "none", "case": kw["case"] or "original", "fills": None, "link": None}
            if kw["tnum"]:
                d["features"] = {"tnum": True}
            d["style"] = st
            d["fills"] = [paint(kw["color"])]
            d["width"] = float(kw["w"] or 100)
            d["height"] = float(kw["size"]) * 1.3
            if kw["align"] != "left":
                d["align_h"] = kw["align"]
            if kw["w"] or kw["sh"] == "fill":
                d["auto_resize"] = "height"
                cl["sizing_h"] = kw["sh"] or "fixed"
                cl["sizing_v"] = "hug"
            else:
                cl["sizing_h"] = "hug"
                cl["sizing_v"] = "hug"
            if kw["opacity"] is not None:
                d["opacity"] = kw["opacity"]
        else:
            d["width"] = float(kw["w"])
            d["height"] = float(kw["h"])
            d["fills"] = [paint(kw["fill"])] if kw.get("fill") else []
            if kw.get("stroke"):
                d["strokes"], d["stroke"] = _stroke(kw["stroke"], kw["sw"])
            if n.type == "rectangle" and kw.get("radius"):
                r = kw["radius"]
                d["corners"] = {"radii": [float(v) for v in (r if isinstance(r, list) else [r] * 4)], "smoothing": 0.6}
            if n.type == "vector":
                d["vector_paths"] = [{"data": kw["path"], "winding": "nonzero"}]
            cl["sizing_h"] = kw.get("sh") or "fixed"
            cl["sizing_v"] = kw.get("sv") or "fixed"
        d["child_layout"] = cl
        d["children"] = []
        self.nodes[str(nid)] = d
        if top:
            self.nodes[str(parent)]["children"].append(nid)
        for c in n.children:
            self._emit(c, nid, depth + 1, parent_dir=kw.get("dir"))
            d["children"].append(c.id)
        if not d["children"] and n.type != "frame":
            del d["children"]
        return nid

    def save(self, path):
        doc = {"format": "figcraft", "version": 1, "imageIndex": [], "name": self.name,
               "pages": self.pages, "nodes": self.nodes, "next_id": self.next, "styles": [],
               "variables": {"collections": [], "variables": []}, "current_page": self.pages[0],
               "document_colors": [], "comments": []}
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
            z.writestr("document.json", json.dumps(doc))
        # relayout plan: deepest first, grouped per page handled by caller
        relayout = [{"command": "node.set", "params": {"ids": [i], "gap": g}}
                    for _, i, g in sorted(self.layout_frames, key=lambda t: -t[0])]
        with open(path + ".relayout.json", "w") as f:
            json.dump(relayout, f)
        return path
