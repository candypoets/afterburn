#!/usr/bin/env node
/**
 * Local EUR display patch for @deepseek-harness-tui/dsh-tui 0.16.x.
 *
 * The TUI prices the DeepSeek session in CNY and renders a hard-coded "¥"
 * (deepseekPricing.js + the status bar, side panel, i18n and balance row).
 * There is no currency setting, so this script rewrites those five files in
 * the installed package:
 *
 *   1. converts the estimate (total/main/subagent/peak/idle) CNY -> EUR,
 *   2. converts the per-million CNY rates shown in the balance hover,
 *   3. swaps the "¥" symbol for "€" in the cost readouts (account balance
 *      stays CNY — that is the real currency of the DeepSeek account).
 *
 * The rate defaults to 0.13 and can be overridden at launch with the
 * DSH_TUI_CNY_EUR environment variable (CNY -> EUR).
 *
 * Usage:
 *   node apply.mjs                     patch the profile's installed package
 *   node apply.mjs --package <dir>     patch another copy
 *   node apply.mjs --rate 0.128        compile a different default rate
 *   node apply.mjs --dry-run           show what would change
 *   node apply.mjs --revert            restore the .eur-backup originals
 *   node apply.mjs --tps               also set statusBar.tps=true in
 *                                      ~/.dsh-tui/settings.json
 *
 * Every changed file is backed up next to itself as "<name>.eur-backup"
 * before writing, and syntax-checked with `node --check` in between. A TUI
 * update reinstalls the package and drops the patch — re-run this script.
 */
import { execFileSync } from 'node:child_process';
import {
    copyFileSync,
    existsSync,
    mkdirSync,
    readFileSync,
    renameSync,
    rmSync,
    writeFileSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';

const DEFAULT_PACKAGE =
    '/root/.dsh/profiles/tui/node_modules/@deepseek-harness-tui/dsh-tui';
const TPS_SETTINGS_FILE = join(homedir(), '.dsh-tui', 'settings.json');

const argv = process.argv.slice(2);
const has = (name) => argv.includes(name);
const value = (name, fallback) => {
    const index = argv.indexOf(name);
    return index === -1 ? fallback : argv[index + 1];
};

const dryRun = has('--dry-run');
const revert = has('--revert');
const withTps = has('--tps');
const rate = Number(value('--rate', process.env.DSH_TUI_CNY_EUR ?? '0.13'));
const packageDir = resolve(value('--package', DEFAULT_PACKAGE));

if (!Number.isFinite(rate) || rate <= 0) {
    fail(`--rate must be a positive number (got ${value('--rate', '')})`);
}

function fail(message) {
    console.error(`error: ${message}`);
    process.exit(1);
}

function info(message) {
    console.log(message);
}

// ---------------------------------------------------------------------------
// Patch definitions
// ---------------------------------------------------------------------------

/** Replace `oldText` exactly `count` times, or throw with a readable message. */
function replaceExactly(text, oldText, newText, count, label) {
    const found = text.split(oldText).length - 1;
    if (found !== count) {
        throw new Error(
            `${label}: expected ${count} occurrence(s) of ${JSON.stringify(
                oldText.slice(0, 60),
            )}, found ${found}`,
        );
    }
    return text.split(oldText).join(newText);
}

/** Replace all "¥" on the single i18n line whose key matches. */
function patchI18nLine(text, key) {
    const lines = text.split('\n');
    let hits = 0;
    const patched = lines.map((line) => {
        if (!line.startsWith(`    '${key}':`)) return line;
        hits += 1;
        return line.replaceAll('¥', '€');
    });
    if (hits !== 1) {
        throw new Error(`i18n: key ${key} matched ${hits} line(s)`);
    }
    return patched.join('\n');
}

const CURRENCY_BLOCK = `/**
 * Display currency for the cost estimate. The official price list is quoted in
 * CNY; amounts are converted for display. Override the rate at launch with
 * DSH_TUI_CNY_EUR (CNY -> EUR).
 */
const RAW_EUR_RATE = Number(process.env.DSH_TUI_CNY_EUR ?? '${rate}');
export const DISPLAY_CURRENCY = Object.freeze({
    code: 'EUR',
    symbol: '€',
    rate: Number.isFinite(RAW_EUR_RATE) && RAW_EUR_RATE > 0 ? RAW_EUR_RATE : ${rate},
});
/** Convert a CNY amount (or a per-million CNY rate) to the display currency. */
export function toDisplayCurrency(cny) {
    return cny * DISPLAY_CURRENCY.rate;
}

`;

const PATCHES = [
    {
        file: 'lib/types/deepseekPricing.js',
        isPatched: (text) => text.includes('toDisplayCurrency'),
        apply: (text) => {
            let next = replaceExactly(
                text,
                'export const DEEPSEEK_MODEL_PRICES = {',
                `${CURRENCY_BLOCK}export const DEEPSEEK_MODEL_PRICES = {`,
                1,
                'deepseekPricing: prices table anchor',
            );
            next = replaceExactly(
                next,
                `    return {
        total: split.peak / 1_000_000 + split.idle / 1_000_000,
        peak: split.peak / 1_000_000,
        idle: split.idle / 1_000_000,
    };`,
                `    return {
        total: toDisplayCurrency(split.peak / 1_000_000 + split.idle / 1_000_000),
        peak: toDisplayCurrency(split.peak / 1_000_000),
        idle: toDisplayCurrency(split.idle / 1_000_000),
    };`,
                1,
                'deepseekPricing: estimateSessionCostSplitCny return',
            );
            next = replaceExactly(
                next,
                '    return { total, main, subagent, peak, idle, unpricedTokens };',
                `    return {
        total: toDisplayCurrency(total),
        main: toDisplayCurrency(main),
        subagent: toDisplayCurrency(subagent),
        peak: toDisplayCurrency(peak),
        idle: toDisplayCurrency(idle),
        unpricedTokens,
    };`,
                1,
                'deepseekPricing: estimateCostFromBucketsCny return',
            );
            return next;
        },
    },
    {
        file: 'lib/types/screens/StatusLine.js',
        isPatched: (text) => text.includes('\\u20AC'),
        apply: (text) => {
            let next = replaceExactly(
                text,
                '\\u00A5',
                '\\u20AC',
                3,
                'StatusLine: escaped yuan symbols',
            );
            next = replaceExactly(
                next,
                "dim('≈¥')",
                "dim('≈€')",
                1,
                'StatusLine: hover label',
            );
            return next;
        },
    },
    {
        file: 'lib/types/components/sidePanel/InfoPanel.js',
        isPatched: (text) => text.includes("'€' + estimate.total"),
        apply: (text) =>
            replaceExactly(
                text,
                "'¥' + estimate.total.toFixed(2)",
                "'€' + estimate.total.toFixed(2)",
                1,
                'InfoPanel: cost symbol',
            ),
    },
    {
        file: 'lib/types/components/BalanceReportRow.js',
        isPatched: (text) => text.includes('toDisplayCurrency'),
        apply: (text) => {
            let next = replaceExactly(
                text,
                "import { estimateSessionCostSnapshotCny, isPeakHour, priceForModel } from '../deepseekPricing.js';",
                "import { estimateSessionCostSnapshotCny, isPeakHour, priceForModel, toDisplayCurrency } from '../deepseekPricing.js';",
                1,
                'BalanceReportRow: pricing import',
            );
            next = replaceExactly(
                next,
                'input: price.inputMiss[rateIndex].toFixed(1),',
                'input: toDisplayCurrency(price.inputMiss[rateIndex]).toFixed(2),',
                1,
                'BalanceReportRow: input rate',
            );
            next = replaceExactly(
                next,
                'output: price.output[rateIndex].toFixed(1),',
                'output: toDisplayCurrency(price.output[rateIndex]).toFixed(2),',
                1,
                'BalanceReportRow: output rate',
            );
            return next;
        },
    },
    {
        file: 'lib/types/i18n.js',
        isPatched: (text) =>
            text.includes(`'cost-session-estimate': { zh: '本会话估算 ≈€`),
        apply: (text) => {
            let next = text;
            for (const key of [
                'cost-session-estimate',
                'cost-split-main',
                'cost-split-subagent',
                'balance-hover-tokens',
                'balance-current-rate',
            ]) {
                next = patchI18nLine(next, key);
            }
            return next;
        },
    },
];

// ---------------------------------------------------------------------------
// File helpers
// ---------------------------------------------------------------------------

function backupPath(file) {
    return `${file}.eur-backup`;
}

/** Write via a temp sibling and `node --check`, then rename over the target. */
function writeChecked(file, content) {
    const temporary = join(
        dirname(file),
        `.${basename(file)}.${process.pid}.eur.tmp.js`,
    );
    writeFileSync(temporary, content, 'utf8');
    try {
        execFileSync(process.execPath, ['--check', temporary], {
            stdio: ['ignore', 'ignore', 'pipe'],
        });
    } catch (error) {
        rmSync(temporary, { force: true });
        const stderr = error.stderr?.toString().trim() ?? String(error);
        throw new Error(`syntax check failed for ${file}:\n${stderr}`);
    }
    renameSync(temporary, file);
}

function readPackageVersion(dir) {
    const manifest = JSON.parse(
        readFileSync(join(dir, 'package.json'), 'utf8'),
    );
    return manifest;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

if (!existsSync(join(packageDir, 'package.json'))) {
    fail(
        `no @deepseek-harness-tui/dsh-tui package at ${packageDir}\n` +
            'pass --package <dir> (resolve the profile symlink first)',
    );
}

const manifest = readPackageVersion(packageDir);
info(`package: ${manifest.name}@${manifest.version} (${packageDir})`);
info(`currency: EUR at 1 CNY = ${rate} EUR`);
if (dryRun) info('mode: dry run (nothing is written)');
if (revert) info('mode: revert');

let changed = 0;
let already = 0;

for (const patch of PATCHES) {
    const file = join(packageDir, patch.file);
    if (!existsSync(file)) {
        fail(`missing file: ${file}`);
    }

    if (revert) {
        const backup = backupPath(file);
        if (!existsSync(backup)) {
            info(`  = ${patch.file} (no backup, left alone)`);
            continue;
        }
        if (!dryRun) copyFileSync(backup, file);
        info(`  ↩ ${patch.file} (restored)`);
        changed += 1;
        continue;
    }

    const original = readFileSync(file, 'utf8');
    if (patch.isPatched(original)) {
        info(`  = ${patch.file} (already patched)`);
        already += 1;
        continue;
    }

    let patched;
    try {
        patched = patch.apply(original);
    } catch (error) {
        fail(`${patch.file}: ${error.message}`);
    }

    if (patched === original) {
        info(`  = ${patch.file} (no change)`);
        continue;
    }

    if (!dryRun) {
        const backup = backupPath(file);
        try {
            if (!existsSync(backup)) writeFileSync(backup, original, 'utf8');
            writeChecked(file, patched);
        } catch (error) {
            fail(
                `${patch.file}: ${error.message}` +
                    (error.code === 'EROFS' || error.code === 'EACCES'
                        ? '\n  this package is on a read-only mount — run the script on the host that owns ~/.dsh'
                        : ''),
            );
        }
    }
    info(`  ${dryRun ? '~' : '✎'} ${patch.file}`);
    changed += 1;
}

if (revert && !dryRun) {
    for (const patch of PATCHES) {
        const backup = backupPath(join(packageDir, patch.file));
        if (existsSync(backup)) rmSync(backup, { force: true });
    }
}

// ---------------------------------------------------------------------------
// statusBar.tps (optional)
// ---------------------------------------------------------------------------

if (withTps && !revert) {
    const file = value('--settings', TPS_SETTINGS_FILE);
    let document = { version: 1, values: {} };
    if (existsSync(file)) {
        try {
            const parsed = JSON.parse(readFileSync(file, 'utf8'));
            if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
                fail(`${file} does not contain a JSON object`);
            }
            document = parsed;
        } catch (error) {
            fail(`cannot parse ${file}: ${error.message}`);
        }
    }
    const values =
        document.values !== null &&
        typeof document.values === 'object' &&
        !Array.isArray(document.values)
            ? document.values
            : {};
    const statusBar =
        values.statusBar !== null &&
        typeof values.statusBar === 'object' &&
        !Array.isArray(values.statusBar)
            ? values.statusBar
            : {};
    const next = {
        ...document,
        version: document.version ?? 1,
        values: { ...values, statusBar: { ...statusBar, tps: true } },
    };
    if (!dryRun) {
        try {
            mkdirSync(dirname(file), { recursive: true });
            writeFileSync(file, `${JSON.stringify(next, null, 2)}\n`, {
                encoding: 'utf8',
                mode: 0o600,
            });
        } catch (error) {
            fail(
                `${file}: ${error.message}` +
                    (error.code === 'EROFS' || error.code === 'EACCES'
                        ? '\n  ~/.dsh-tui is on a read-only mount — run the script on the host that owns it'
                        : ''),
            );
        }
    }
    info(
        `  ${dryRun ? '~' : '✎'} ${file} (statusBar.tps = true)\n` +
            '    note: a running TUI keeps its own copy — prefer /settings while it is open',
    );
    changed += 1;
}

info('');
if (revert) {
    info(
        changed === 0
            ? 'nothing to revert (no .eur-backup files)'
            : `reverted ${changed} file(s); restart the TUI to load the originals`,
    );
} else if (dryRun) {
    info(`dry run: ${changed} file(s) would change, ${already} already patched`);
} else {
    info(
        changed === 0
            ? 'nothing to do — the package is already patched'
            : `patched ${changed} file(s), ${already} already up to date\n` +
                  'restart dsh tui to load the new currency',
    );
}
