PY ?= python3

.PHONY: check anchors build publish

check:
	$(PY) design/build_map.py --check

anchors:
	$(PY) design/check_anchors.py

build:
	$(PY) design/build_map.py

publish:
	$(PY) design/build_map.py --publish
