# Session cost in EUR for dsh-tui

`@deepseek-harness-tui/dsh-tui` 0.16.1 estimates the DeepSeek session spend in
CNY and hard-codes `¥` in every readout — status bar, hover, `/cost`, side
panel and `/balance`. There is **no currency setting**. This directory patches
the installed package so those readouts show EUR.

What the patch does:

- converts the estimate (`total` / `main` / `subagent` / `peak` / `idle`)
  from CNY to EUR at one choke point in `lib/types/deepseekPricing.js`;
- converts the per-million rates printed in the `/balance` hover;
- swaps `¥` for `€` in the cost strings (status bar, hover, `/cost`, side
  panel). The **account balance keeps `¥`** — that is the real currency of a
  DeepSeek account, not an estimate.

Files touched (each backed up as `<name>.eur-backup` before writing and
syntax-checked with `node --check`):

```
lib/types/deepseekPricing.js
lib/types/screens/StatusLine.js
lib/types/components/sidePanel/InfoPanel.js
lib/types/components/BalanceReportRow.js
lib/types/i18n.js
```

## Apply

The package lives in the `tui` profile's `node_modules`:

```sh
node tools/dsh-tui-eur/apply.mjs --dry-run     # show what would change
node tools/dsh-tui-eur/apply.mjs               # patch, then restart dsh tui
```

Run it as the user that owns `~/.dsh` (use `sudo` only if that profile is
root-owned). Point it elsewhere with `--package <dir>`:

```sh
node tools/dsh-tui-eur/apply.mjs --package ~/.dsh/profiles/tui/node_modules/@deepseek-harness-tui/dsh-tui
```

The exchange rate defaults to `0.13` EUR per CNY. Change the compiled default
with `--rate`, or override it at launch without repatching:

```sh
DSH_TUI_CNY_EUR=0.128 dsh tui
```

## Toggle the TPS field

`statusBar.tps` (the gauge/sparkline/`N tps` readout) is a normal setting and
needs no patch — `/settings` → **Status bar** → **Show output speed**. The
script can also write it directly:

```sh
node tools/dsh-tui-eur/apply.mjs --tps
```

That merges `statusBar.tps = true` into `~/.dsh-tui/settings.json`. Prefer the
`/settings` toggle while a TUI is running: the live process keeps its own copy
of the document and can overwrite the file on the next settings change.

## Revert

```sh
node tools/dsh-tui-eur/apply.mjs --revert
```

Restores every `.eur-backup` and removes the backups. A TUI update or a
profile reinstall replaces the package and drops the patch; re-run the script
after updating.

## Caveats

- This edits `node_modules` internals, not a supported API. It is pinned to
  the 0.16.x layout — the script refuses to guess if a file or string changed
  (it fails loudly instead of half-patching).
- The estimate was already approximate (local token counts × official rates);
  converting it adds a fixed-rate approximation on top. The platform bill
  remains authoritative.
- `estimateSessionCostCny` and friends keep their names but now return the
  display currency. Nothing else in the package imports their values except
  the renderers listed above.
