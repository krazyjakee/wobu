/* ── domain types ─────────────────────────────────────────────────────────── */

export type NodeKind =
  | 'style_guide'
  | 'world_bible'
  | 'species'
  | 'culture'
  | 'setting'
  | 'character'
  | 'creature'
  | 'prop'
  | 'environment'
  | 'vehicle'

export type DescriptionState = 'none' | 'enhancing' | 'fresh' | 'edited' | 'stale'

export type LinkRole = 'species_of' | 'member_of' | 'located_in' | 'styled_by' | 'related_to'

export type SectionValue = { type: 'text'; value: string } | { type: 'list'; value: string[] }

export type SectionValueKind = 'text' | 'list'

export type AttributeValueKind = 'text' | 'number' | 'boolean'

/** `key` indexes `WobuNode.attributes`; the value kind selects the generated control. */
export interface AttributeDef {
  key: string
  label: string
  valueKind: AttributeValueKind
}

/** `key` indexes `WobuNode.description.sections`; `label` is the rendered heading. */
export interface SectionDef {
  key: string
  label: string
  valueKind: SectionValueKind
}

export type InfluenceLayer =
  'style' | 'world' | 'ancestry' | 'culture' | 'place' | 'subject' | 'shot'

export interface KindDef {
  kind: NodeKind
  label: string
  plural: string
  icon: string
  color: string
  layer: InfluenceLayer
  /** folder under nodes/ — not used by the UI */
  dir: string
  nests: boolean
  singleton: boolean
  /** Kind-specific facts rendered as controls in the Notes tab. */
  attributes: AttributeDef[]
  /** already in the kind's intended display order */
  sections: SectionDef[]
  defaultLinkRoles: LinkRole[]
}

export interface ProjectSummary {
  id: string
  name: string
  path: string
  onNetworkShare: boolean
  readOnly: boolean
  lastOpenedAt: string | null
}

export interface Link {
  toId: string
  role: LinkRole
  weight: number
  enabled: boolean
}

/** An indexed link with both endpoints, used by the Relations backlinks list. */
export interface LinkEdge extends Link {
  fromId: string
}

/**
 * What a reference image is *for*, and therefore where it is routed when a
 * generation is compiled — a `palette` reference goes to colour conditioning, a
 * `pose` reference to a structure adapter.
 *
 * `mood` is the exception worth knowing: it is `moodboard_only`, shown to the
 * human and never sent anywhere. The backend enforces that; nothing on this
 * side should be written that assumes otherwise.
 */
export type AssetRole =
  'silhouette' | 'palette' | 'material' | 'mood' | 'pose' | 'costume' | 'full_ref'

export const ASSET_ROLES: AssetRole[] = [
  'silhouette',
  'palette',
  'material',
  'mood',
  'pose',
  'costume',
  'full_ref',
]

/**
 * A reference image attached to a node.
 *
 * `(assetId, role)` is the identity: the same picture can be both a `full_ref`
 * and a `palette` source for one entity, because those reach two different
 * adapters. Weight is 0.0–1.0, defaulting to 1.0, exactly as on `Link`.
 */
export interface AssetLink {
  assetId: string
  role: AssetRole
  weight: number
  /** Muted for now, without detaching it. */
  enabled: boolean
}

/** One immutable, project-owned entity fine-tune pinned by its content hash. */
export interface LoraPin {
  hash: string
  relPath: string
  bytes: number
  trainer: string
  protocol: number
  baseModel: string
  modelFamily: string
  providerName: string
  triggerToken: string
  inputAssetHashes: string[]
  createdAt: string
  strength: number
}

export interface NodeSummary {
  id: string
  kind: NodeKind
  name: string
  summary: string
  parentId: string | null
  descriptionState: DescriptionState
  slug: string
}

export interface WobuNode {
  id: string
  kind: NodeKind
  name: string
  slug: string
  summary: string
  parentId: string | null
  notesRaw: string
  description: { sections: Record<string, SectionValue> } | null
  descriptionState: DescriptionState
  attributes: Record<string, unknown>
  tags: string[]
  /** The image shown on this entity's card. Independent of `assetLinks`. */
  coverAssetId: string | null
  /** Shared identity seed used whenever Generate has no explicit re-roll. */
  lockedSeed: number | null
  /** Optional local entity fine-tune, applied only when its model is compatible. */
  lora: LoraPin | null
  links: Link[]
  assetLinks: AssetLink[]
  createdAt: string
  updatedAt: string
}

/**
 * What a blob is for. Roles (`palette`, `pose`, …) live on the link between an
 * asset and a node; this is the coarser question of where the file came from.
 */
export type AssetKind = 'reference' | 'generated' | 'upload'

/**
 * A file in `assets/originals/`, addressed by the hash of its contents.
 *
 * Both `id` and `relPath` are derived from that hash and from nothing else, so
 * two people importing the same picture on one share produce the same file
 * *and* the same record — and neither survives being renamed, because neither
 * ever depended on a name. `id` is what `coverAssetId` and every AssetLink
 * point at.
 */
export interface Asset {
  id: string
  /** Lowercase hex BLAKE3 of the file. This, not the id, names the file. */
  hash: string
  kind: AssetKind
  /** Project-relative, `/`-separated. Never absolute. */
  relPath: string
  /** Null until something has actually made a thumbnail. */
  thumbPath: string | null
  /** Read out of the file's own header, not out of its extension. */
  mime: string
  width: number
  height: number
  bytes: number
  createdAt: string
}

export interface AssetUsageRole {
  role: AssetRole
  weight: number
  enabled: boolean
}

/** One node's complete use of an asset, including its independent cover use. */
export interface AssetUsage {
  assetId: string
  nodeId: string
  nodeName: string
  nodeKind: NodeKind
  /** Canonical tags on the linked node; assets have no mutable tag document. */
  nodeTags: string[]
  roles: AssetUsageRole[]
  cover: boolean
}

/* ── environment ──────────────────────────────────────────────────────────── */
