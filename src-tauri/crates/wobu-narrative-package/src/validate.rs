use crate::{
    FORMAT_VERSION, MAX_FILE_BYTES, MAX_STRING_BYTES, MAX_TOTAL_BYTES, Manifest, REQUIRED, Result,
    hash, invalid, json,
};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use wobu_narrative::{Condition, Effect, EntityId, StateSchema, VarType, VariableDecl};
use wobu_narrative_compiler::{GRAPH_VERSION, Graph, Profile, Target, argument_fits};

pub fn manifest(manifest: &Manifest) -> Result<()> {
    if manifest.format != "wobu-narrative"
        || manifest.version != FORMAT_VERSION
        || manifest.graph_version != GRAPH_VERSION
        || manifest.locale != "en"
    {
        return Err(invalid("unsupported format, schema, graph version or locale"));
    }
    let capabilities = BTreeMap::from([
        ("deterministic_graph".to_string(), 1),
        ("separate_strings".to_string(), 1),
    ]);
    if manifest.required_capabilities != capabilities {
        return Err(invalid("unsupported or missing required capabilities"));
    }
    if REQUIRED.iter().any(|name| !manifest.files.contains_key(*name)) {
        return Err(invalid("required payload file missing"));
    }
    let mut total = 0u64;
    for (path, record) in &manifest.files {
        portable_path(path)?;
        if !REQUIRED.contains(&path.as_str())
            && !(path == "debug/source-map.json" && manifest.profile == Profile::Development)
        {
            return Err(invalid(format!("unexpected payload path {path}")));
        }
        if record.bytes > MAX_FILE_BYTES {
            return Err(invalid(format!("file exceeds size limit: {path}")));
        }
        total = total.checked_add(record.bytes).ok_or_else(|| invalid("total size overflow"))?;
        if record.hash.len() != 64
            || !record.hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("invalid content hash"));
        }
    }
    if total > MAX_TOTAL_BYTES || manifest.payload_hash != hash(&json(&manifest.files)?) {
        return Err(invalid("payload identity or total size invalid"));
    }
    Ok(())
}

pub fn portable_path(path: &str) -> Result<()> {
    if path.len() > 180
        || path.is_empty()
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
                || part.ends_with('.')
        })
    {
        return Err(invalid(format!("unsafe portable path {path}")));
    }
    Ok(())
}

pub fn graph(graph: &Graph) -> Result<()> {
    if graph.version != GRAPH_VERSION || graph.scenes.is_empty() {
        return Err(invalid("unsupported or empty graph"));
    }
    for variable in graph.state.values() {
        domain(&variable.ty)?;
    }
    for signature in graph.commands.values() {
        for ty in signature {
            domain(ty)?;
        }
    }
    let schema = StateSchema::new(graph.state.iter().map(|(name, decl)| VariableDecl {
        name: name.clone(),
        ty: decl.ty.clone(),
        default: decl.default.clone(),
        owner: decl.owner,
        description: String::new(),
    }))
    .map_err(|e| invalid(e.to_string()))?;
    let mut ids = BTreeSet::new();
    let mut add = |id: &str| -> Result<()> {
        let parsed =
            id.parse::<EntityId>().map_err(|_| invalid(format!("invalid stable id {id}")))?;
        if parsed.to_string() != id {
            return Err(invalid(format!("non-canonical stable id {id}")));
        }
        if !ids.insert(id.to_string()) {
            return Err(invalid(format!("duplicate stable id {id}")));
        }
        Ok(())
    };
    for (scene_id, scene) in &graph.scenes {
        add(scene_id)?;
        if !scene.beats.contains_key(&scene.first) {
            return Err(invalid("missing scene entry beat"));
        }
        condition(&schema, scene.entry.as_ref())?;
        for (beat_id, beat) in &scene.beats {
            add(beat_id)?;
            if beat.choices.is_empty() && beat.outcomes.is_empty() {
                return Err(invalid("beat has no destination"));
            }
            for slot in &beat.dialogue {
                add(&slot.id)?;
                if slot.variants.is_empty() && graph.profile == Profile::Release {
                    return Err(invalid("release slot missing text"));
                }
                for variant in &slot.variants {
                    add(&variant.id)?;
                    condition(&schema, variant.when.as_ref())?;
                    text(&variant.text, graph.profile)?;
                    if variant.revision.len() != 32
                        || !variant.revision.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(invalid("invalid wording revision"));
                    }
                }
            }
            for choice in &beat.choices {
                add(&choice.id)?;
                text(&choice.label, graph.profile)?;
                condition(&schema, choice.requires.as_ref())?;
            }
            for outcome in &beat.outcomes {
                add(&outcome.id)?;
                condition(&schema, outcome.when.as_ref())?;
            }
            for (effects, to) in beat
                .choices
                .iter()
                .map(|c| (&c.effects, &c.to))
                .chain(beat.outcomes.iter().map(|o| (&o.effects, &o.to)))
            {
                match to {
                    Target::Beat(id) if !scene.beats.contains_key(id) => {
                        return Err(invalid("missing target beat"));
                    }
                    Target::Scene(id) if !graph.scenes.contains_key(id) => {
                        return Err(invalid("missing target scene"));
                    }
                    _ => {}
                }
                for effect in effects {
                    schema.check_effect(effect).map_err(|e| invalid(e.to_string()))?;
                    if let Effect::Command(command) = effect {
                        let signature = graph
                            .commands
                            .get(&command.name)
                            .ok_or_else(|| invalid("unregistered command"))?;
                        if signature.len() != command.args.len()
                            || !signature
                                .iter()
                                .zip(&command.args)
                                .all(|(ty, arg)| argument_fits(ty, arg, &schema))
                        {
                            return Err(invalid("invalid command argument"));
                        }
                    }
                }
            }
        }
    }
    for (id, source) in &graph.source_map {
        if !ids.contains(id) {
            return Err(invalid("debug map references unknown element"));
        }
        let scene = graph
            .scenes
            .get(&source.scene)
            .ok_or_else(|| invalid("debug map references unknown scene"))?;
        if let Some(beat) = &source.beat {
            let beat = scene
                .beats
                .get(beat)
                .ok_or_else(|| invalid("debug map references unknown beat"))?;
            if source
                .slot
                .as_ref()
                .is_some_and(|id| !beat.dialogue.iter().any(|slot| &slot.id == id))
            {
                return Err(invalid("debug map references unknown slot"));
            }
        } else if source.slot.is_some() {
            return Err(invalid("debug slot has no beat"));
        }
    }
    Ok(())
}
fn text(text: &str, profile: Profile) -> Result<()> {
    if text.len() > MAX_STRING_BYTES || (profile == Profile::Release && text.trim().is_empty()) {
        return Err(invalid("text is empty for release or exceeds size limit"));
    }
    Ok(())
}
fn condition(schema: &StateSchema, condition: Option<&Condition>) -> Result<()> {
    if let Some(condition) = condition {
        schema.check_condition(condition).map_err(|e| invalid(e.to_string()))?;
    }
    Ok(())
}
fn domain(ty: &VarType) -> Result<()> {
    match ty {
        VarType::Int { min, max } if min > max => Err(invalid("empty integer domain")),
        VarType::Enum { members }
            if members.is_empty()
                || members.iter().collect::<BTreeSet<_>>().len() != members.len() =>
        {
            Err(invalid("empty or duplicate enum domain"))
        }
        _ => Ok(()),
    }
}
pub(crate) struct UniqueJson(pub serde_json::Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Unique;
        impl<'de> Visitor<'de> for Unique {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<Self::Value, E> {
                Err(E::custom("fractional/exponential JSON numbers are not supported"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueJson(serde_json::Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(value) = seq.next_element::<UniqueJson>()? {
                    out.push(value.0);
                }
                Ok(UniqueJson(out.into()))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if out.contains_key(&key) {
                        return Err(de::Error::custom(format!("duplicate JSON key {key}")));
                    }
                    out.insert(key, map.next_value::<UniqueJson>()?.0);
                }
                Ok(UniqueJson(out.into()))
            }
        }
        deserializer.deserialize_any(Unique)
    }
}
