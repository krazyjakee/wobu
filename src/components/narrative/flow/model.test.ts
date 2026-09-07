import { describe, expect, it } from 'vitest'
import { councilHearing } from './fixture'
import {
  addElement,
  connectPort,
  nextElementId,
  removeElement,
  sceneDiagnostics,
  unresolvedPorts,
} from './model'

describe('the Flow view model', () => {
  it('keeps a beat to counts, never to its text', () => {
    const beat = councilHearing().elements.find((element) => element.id === 'beat.2')
    expect(beat?.kind).toBe('beat')
    if (beat?.kind !== 'beat') throw new Error('unreachable')
    // Twelve lines and seven variants, and still one object with no lines in
    // it. If a `lines: FlowLine[]` ever appears, this is the test that says so.
    expect(beat.lines).toBe(12)
    expect(beat.variants).toBe(7)
    expect(Object.keys(beat)).not.toContain('text')
    expect(beat.out).toHaveLength(1)
  })

  it('reports nothing wrong with a scene whose routes all lead somewhere', () => {
    expect(sceneDiagnostics(councilHearing())).toEqual([])
  })

  it('leaves an unresolved-destination diagnostic naming the field when a port is disconnected', () => {
    const scene = connectPort(councilHearing(), { elementId: 'choice.1', portId: 'option.2' }, null)
    const found = sceneDiagnostics(scene)
    expect(found).toHaveLength(1)
    expect(found[0]?.severity).toBe('error')
    expect(found[0]?.field).toBe('choice.1.option.2')
    expect(found[0]?.message).toMatch(/Challenge the captain/)
  })

  it('turns every route into a deleted element into its own unresolved destination', () => {
    // Not "and the edges go too". Three approaches converged on the verdict;
    // deleting it must leave three questions, not silence.
    const scene = removeElement(councilHearing(), 'beat.7')
    const found = sceneDiagnostics(scene)
    expect(found.map((d) => d.field).sort()).toEqual([
      'beat.5.then',
      'beat.6.then',
      'outcome.2.then',
      'outcome.3.then',
    ])
  })

  it('connects an outcome port to a beat, and back to nothing', () => {
    const scene = councilHearing()
    const rerouted = connectPort(scene, { elementId: 'outcome.2', portId: 'then' }, 'end.1')
    const outcome = rerouted.elements.find((element) => element.id === 'outcome.2')
    expect(outcome?.out[0]?.to).toBe('end.1')
    // The original is untouched: every operation returns a new scene.
    expect(scene.elements.find((e) => e.id === 'outcome.2')?.out[0]?.to).toBe('beat.7')
  })

  it('gives a new element the ports its kind requires', () => {
    const scene = councilHearing()
    expect(addElement(scene, 'condition', null).element.out.map((p) => p.id)).toEqual([
      'true',
      'false',
    ])
    expect(addElement(scene, 'end', null).element.out).toEqual([])
    expect(addElement(scene, 'beat', null).element.out.map((p) => p.id)).toEqual(['then'])
  })

  it('wires a new element in through a spare port, never over an existing route', () => {
    const scene = councilHearing()
    // beat.4's only way out already leads to outcome.3, so nothing is displaced.
    const full = addElement(scene, 'beat', 'beat.4')
    expect(full.scene.elements.find((e) => e.id === 'beat.4')?.out[0]?.to).toBe('outcome.3')

    // Free one up first, and the new beat lands in it.
    const opened = connectPort(scene, { elementId: 'beat.4', portId: 'then' }, null)
    const wired = addElement(opened, 'beat', 'beat.4')
    expect(wired.scene.elements.find((e) => e.id === 'beat.4')?.out[0]?.to).toBe(wired.element.id)
  })

  it('mints ids from the scene rather than from a counter or a random source', () => {
    const scene = councilHearing()
    expect(nextElementId(scene, 'beat')).toBe('beat.8')
    const once = addElement(scene, 'beat', null)
    expect(once.element.id).toBe('beat.8')
    expect(addElement(once.scene, 'beat', null).element.id).toBe('beat.9')
  })

  it('knows which ports lead nowhere, for the keyboard connector to pick', () => {
    const scene = connectPort(councilHearing(), { elementId: 'choice.1', portId: 'option.3' }, null)
    const choice = scene.elements.find((element) => element.id === 'choice.1')!
    expect(unresolvedPorts(choice).map((port) => port.id)).toEqual(['option.3'])
  })
})
