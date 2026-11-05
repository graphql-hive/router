#!/usr/bin/env node
// Two runs side by side:
// - `node compare.js out/main/report.json out/branch/report.json`, two `run.js` reports of the same
//   operation, case by case
// - `node compare.js out/fuzz/main out/fuzz/branch`, two `fuzz.js` runs, the cases whose outcome
//   differs, by fixture

import { existsSync, readFileSync, statSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';

import { problemsOf } from './src/analyze.js';
import { outName } from './src/generated.js';

const [leftPath, rightPath] = process.argv.slice(2);
if (!leftPath || !rightPath) {
  console.error('usage: node compare.js <report.json|fuzz dir> <report.json|fuzz dir>');
  process.exit(2);
}
const read = (path) => JSON.parse(readFileSync(path, 'utf8'));

if (statSync(leftPath).isDirectory()) {
  compareFuzz(leftPath, rightPath);
} else {
  compareReports(leftPath, rightPath);
}

function compareReports(leftPath, rightPath) {
  const left = read(leftPath);
  const right = read(rightPath);
  const problems = (result) => problemsOf(result).join(', ') || 'ok';
  const cell = (result) =>
    `${result.requests} req, ${result.lookups} lookups (${result.repeatedLookups} rep), ${problems(result)}`;

  console.log(`${left.operation}: ${basename(dirname(leftPath))} -> ${basename(dirname(rightPath))}`);
  const rightByName = new Map(right.results.map((result) => [result.name, result]));
  const totals = { left: { requests: 0, lookups: 0 }, right: { requests: 0, lookups: 0 } };
  for (const result of left.results) {
    const other = rightByName.get(result.name);
    if (!other) continue;
    totals.left.requests += result.requests;
    totals.left.lookups += result.lookups;
    totals.right.requests += other.requests;
    totals.right.lookups += other.lookups;
    console.log(`  ${result.name.padEnd(24)} ${cell(result).padEnd(60)} -> ${cell(other)}`);
  }
  console.log(
    `  ${'total'.padEnd(24)} ${`${totals.left.requests} req, ${totals.left.lookups} lookups`.padEnd(60)} -> ` +
      `${totals.right.requests} req, ${totals.right.lookups} lookups`,
  );
}

/** What every operation of a fuzz run came to, `{ fixture: { seed: outcome } }`, from its summary. */
function outcomes(dir) {
  const summary = read(join(dir, 'summary.json'));
  const byFixture = {};
  for (const row of summary.summary) {
    if (!row.cases) {
      console.error(`${dir} has no case list, it's from an older fuzz.js. Run it again`);
      process.exit(2);
    }
    byFixture[row.fixture] = row.skipped ? null : row.cases;
  }
  return { faults: summary.faults, byFixture };
}

/** The problems of a case that didn't pass, from its file. */
function problemsAt(dir, fixture, seed) {
  const fixtureDir = join(dir, outName(fixture));
  for (const file of [`seed-${seed}.json`, `seed-${seed}-faults.json`]) {
    if (existsSync(join(fixtureDir, file))) return read(join(fixtureDir, file)).problems.join(', ');
  }
  return '';
}

// Only cases both runs ran are compared. A case one of them didn't run is "not run", never fixed
function compareFuzz(leftDir, rightDir) {
  const left = outcomes(leftDir);
  const right = outcomes(rightDir);
  const [leftName, rightName] = [basename(leftDir), basename(rightDir)];
  if (left.faults !== right.faults) console.log(`note: faults ${left.faults} in ${leftName}, ${right.faults} in ${rightName}`);

  const counts = { same: 0, changed: 0, notRun: 0 };
  for (const fixture of [...new Set([...Object.keys(left.byFixture), ...Object.keys(right.byFixture)])].sort()) {
    const a = left.byFixture[fixture];
    const b = right.byFixture[fixture];
    if (!a || !b) {
      const missing = [!a && leftName, !b && rightName].filter(Boolean).join(' and ');
      const cases = Object.keys(a ?? b ?? {}).length;
      counts.notRun += cases;
      console.log(`${fixture}\n  not run in ${missing}${cases ? ` (${cases} cases)` : ''}`);
      continue;
    }
    const lines = [];
    for (const seed of [...new Set([...Object.keys(a), ...Object.keys(b)])].sort((x, y) => x - y)) {
      if (!(seed in a) || !(seed in b)) {
        counts.notRun++;
        lines.push(`  seed-${seed}: not run in ${seed in a ? rightName : leftName}`);
      } else if (a[seed] === b[seed]) {
        counts.same++;
      } else {
        counts.changed++;
        const detail = (dir, outcome) => (outcome === 'passed' ? outcome : `${outcome} (${problemsAt(dir, fixture, seed)})`);
        lines.push(`  seed-${seed}: ${detail(leftDir, a[seed])} -> ${detail(rightDir, b[seed])}`);
      }
    }
    if (lines.length) console.log(`${fixture}\n${lines.join('\n')}`);
  }
  console.log(`\n${leftName} -> ${rightName}: ${counts.same} same, ${counts.changed} changed, ${counts.notRun} not run in one of them`);
}
