import type { KindDef, NodeKind, NodeSummary, WobuNode } from '../lib/api'

/**
 * Builders, not fixtures-as-constants: a test that spells out only the two
 * fields it cares about is a test whose intent survives a schema change.
 */

export function summary(over: Partial<NodeSummary> & { id: string }): NodeSummary {
  return {
    kind: 'character',
    name: over.id,
    summary: '',
    parentId: null,
    descriptionState: 'none',
    slug: over.id,
    ...over,
  }
}

export function kindDef(kind: NodeKind, over: Partial<KindDef> = {}): KindDef {
  return {
    kind,
    label: kind,
    plural: `${kind}s`,
    icon: '',
    color: '',
    layer: 'subject',
    dir: kind,
    nests: true,
    singleton: false,
    sections: [],
    defaultLinkRoles: [],
    ...over,
  }
}

export function kindIndex(defs: KindDef[]): Map<NodeKind, KindDef> {
  return new Map(defs.map((d) => [d.kind, d]))
}

export function node(over: Partial<WobuNode> & { id: string }): WobuNode {
  return {
    kind: 'character',
    name: over.id,
    slug: over.id,
    summary: '',
    parentId: null,
    notesRaw: '',
    description: null,
    descriptionState: 'none',
    attributes: {},
    tags: [],
    coverAssetId: null,
    links: [],
    assetLinks: [],
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
    ...over,
  }
}
