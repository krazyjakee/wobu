import assert from 'node:assert/strict'
import { test } from 'node:test'

import { LEVEL, audit, levelOfExpression } from './generate-third-party-notices.mjs'

/*
 * The licence classifier, which is the half of the notice generator that can be
 * wrong quietly. A missed copyleft term ships in a release; a false positive is
 * a red CI check on a dependency that was always fine, and the cure for that is
 * usually to loosen the rule, so both directions have to be pinned.
 *
 * The expressions below are real ones from `src-tauri/Cargo.lock`, not invented
 * shapes: the crates.io graph is where the awkward cases actually come from.
 */

test('attribution-only licences pass', () => {
  for (const expression of [
    'MIT',
    'Apache-2.0',
    'ISC',
    'Zlib',
    'Unlicense',
    '0BSD',
    'BSD-3-Clause',
    'CC0-1.0',
    'Unicode-3.0',
    'BSL-1.0',
  ]) {
    assert.equal(levelOfExpression(expression), LEVEL.OK, expression)
  }
})

test('a disjunction is worth its cheapest branch', () => {
  // The case that matters most: a third of the crates.io graph offers a
  // copyleft option alongside a permissive one, and a substring scan would
  // report every one of them.
  assert.equal(levelOfExpression('MIT OR Apache-2.0 OR LGPL-2.1-or-later'), LEVEL.OK)
  assert.equal(levelOfExpression('Apache-2.0 OR GPL-2.0-only'), LEVEL.OK)
  assert.equal(levelOfExpression('GPL-3.0-only OR MIT'), LEVEL.OK)
  // Pre-SPDX cargo syntax for the same thing.
  assert.equal(levelOfExpression('MIT/Apache-2.0'), LEVEL.OK)
  assert.equal(levelOfExpression('Apache-2.0 / MIT'), LEVEL.OK)
})

test('a conjunction is worth its dearest term', () => {
  assert.equal(levelOfExpression('Apache-2.0 AND MIT'), LEVEL.OK)
  assert.equal(levelOfExpression('MIT AND GPL-3.0-only'), LEVEL.BLOCKED)
  assert.equal(levelOfExpression('(MIT OR Apache-2.0) AND Unicode-3.0'), LEVEL.OK)
  assert.equal(levelOfExpression('(MIT OR Apache-2.0) AND AGPL-3.0-only'), LEVEL.BLOCKED)
  assert.equal(
    levelOfExpression(
      'ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND ' +
        '(Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0)',
    ),
    LEVEL.OK,
  )
})

test('reciprocal and non-commercial terms are blocked outright', () => {
  for (const expression of [
    'GPL-2.0-only',
    'GPL-3.0-or-later',
    'AGPL-3.0-only',
    'SSPL-1.0',
    'CC-BY-NC-4.0',
    'BUSL-1.1',
    'PolyForm-Noncommercial-1.0.0',
    'Elastic-2.0',
  ]) {
    assert.equal(levelOfExpression(expression), LEVEL.BLOCKED, expression)
  }
})

test('weak copyleft and unrecognised terms are raised, not blocked', () => {
  // MPL-2.0 is in this graph today: six crates, all linked unmodified, which is
  // a thing to know about rather than a thing to fail a build over.
  assert.equal(levelOfExpression('MPL-2.0'), LEVEL.REVIEW)
  assert.equal(levelOfExpression('LGPL-3.0-only'), LEVEL.REVIEW)
  assert.equal(levelOfExpression('EPL-2.0'), LEVEL.REVIEW)
  assert.equal(levelOfExpression('CC-BY-4.0'), LEVEL.REVIEW)
  assert.equal(levelOfExpression('SomeoneElsesLicence-1.0'), LEVEL.REVIEW)
  // A package that declares nothing is the same question as one that declares
  // something nobody recognises.
  assert.equal(levelOfExpression(null), LEVEL.REVIEW)
  assert.equal(levelOfExpression(''), LEVEL.REVIEW)
})

test('a linking exception softens an otherwise viral licence', () => {
  assert.equal(levelOfExpression('GPL-2.0 WITH Classpath-exception-2.0'), LEVEL.REVIEW)
  assert.equal(levelOfExpression('GPL-3.0 WITH GCC-exception-3.1'), LEVEL.REVIEW)
  // Apache's LLVM exception is on a licence that was never a problem.
  assert.equal(levelOfExpression('Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT'), LEVEL.OK)
})

test('the audit reports only what needs an answer, worst first', () => {
  const findings = audit([
    { name: 'fine', version: '1.0.0', licence: 'MIT OR Apache-2.0' },
    { name: 'weak', version: '2.0.0', licence: 'MPL-2.0' },
    { name: 'viral', version: '3.0.0', licence: 'AGPL-3.0-only' },
    { name: 'silent', version: '4.0.0', licence: null },
  ])

  assert.deepEqual(
    findings.map((f) => [f.name, f.level]),
    [
      ['viral', LEVEL.BLOCKED],
      ['silent', LEVEL.REVIEW],
      ['weak', LEVEL.REVIEW],
    ],
  )
  assert.equal(findings[1].licence, '— not declared —')
})
