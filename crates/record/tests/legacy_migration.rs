//! WP-A02 legacy scalar migration against the checked-in fixture produced by
//! the historical code at c8aa4671 (see tests/fixtures/legacy-scalar-c8aa467).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use neuradix_contracts::layout::WireLayout;
use neuradix_contracts::{load_file, schema_identity};
use neuradix_record::legacy_scalar::{
    LEGACY_CODEC_LABEL, LEGACY_PRODUCER_REVISION, LegacyField, LegacyLayout, LegacyScalar,
    MAX_CONTRACT_SOURCE_BYTES, MAX_PROVENANCE_BYTES, MIGRATION_SOFTWARE, PayloadError,
};
use neuradix_record::*;
use neuradix_time::{ClockDomain, Timestamp};
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy-scalar-c8aa467")
}
fn read(name: &str) -> String {
    std::fs::read_to_string(dir().join(name)).unwrap()
}
fn fixture_bytes() -> Vec<u8> {
    let hex: String = read("recording.nrec.hex").split_whitespace().collect();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
fn fixture() -> NativeRecording {
    NativeRecording::from_bytes(&fixture_bytes()).unwrap()
}
fn expected() -> Value {
    serde_json::from_str(&read("expected.json")).unwrap()
}
fn provenance() -> LegacyProvenance {
    LegacyProvenance::from_json(read("provenance.json").as_bytes()).unwrap()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Independent little-endian encoder driven by field types and JSON values.
fn put(out: &mut Vec<u8>, ty: &str, v: &Value) {
    match ty {
        "float64" => out.extend_from_slice(&v.as_f64().unwrap().to_le_bytes()),
        "float32" => out.extend_from_slice(&(v.as_f64().unwrap() as f32).to_le_bytes()),
        "int32" => out.extend_from_slice(&(v.as_i64().unwrap() as i32).to_le_bytes()),
        "int64" => out.extend_from_slice(&v.as_i64().unwrap().to_le_bytes()),
        "uint32" => out.extend_from_slice(&(v.as_u64().unwrap() as u32).to_le_bytes()),
        "uint64" => out.extend_from_slice(&v.as_u64().unwrap().to_le_bytes()),
        "bool" => out.push(u8::from(v.as_bool().unwrap())),
        other => panic!("type {other}"),
    }
}
fn legacy_encode(order: &[LegacyField], values: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    for f in order {
        put(&mut out, &f.ty, &values[&f.name]);
    }
    out
}
fn v2_encode(layout: &WireLayout, values: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    for f in &layout.fields {
        assert_eq!(out.len(), f.offset);
        put(&mut out, &f.ty, &values[&f.name]);
    }
    out
}
fn layout_of(name: &str) -> WireLayout {
    WireLayout::for_contract(&load_file(&dir().join(name)).unwrap()).unwrap()
}
fn channel_contract(id: u16) -> &'static str {
    match id {
        1 => "contract.yaml",
        3 => "depth-reversed.yaml",
        _ => unreachable!(),
    }
}
fn domain(name: &str) -> ClockDomain {
    ClockDomain::parse(name).unwrap()
}

#[test]
fn l1_fixture_is_historical_and_carries_no_wire_metadata() {
    let rec = fixture();
    let m = rec.manifest();
    // Container version 1 and absent wire metadata say nothing about payloads.
    assert_eq!(m.format_version, 1);
    assert!(m.channels.iter().all(|c| c.wire.is_none()));
    assert_eq!(m.writer, "neuradix-record/legacy-fixture");
    assert_eq!(m.seed, Some(7));
    let prov = provenance();
    assert_eq!(prov.producer.revision, LEGACY_PRODUCER_REVISION);
    for entry in &prov.channels {
        let path = dir().join(channel_contract(entry.channel_id));
        let schema = schema_identity(&load_file(&path).unwrap());
        assert_eq!(schema.as_str(), entry.schema_id);
        assert_eq!(
            m.channel(entry.channel_id).unwrap().schema_id,
            entry.schema_id
        );
        assert_eq!(entry.contract, std::fs::read_to_string(path).unwrap());
    }
    // The historical generated sources name the same identities and order.
    let probe = read("legacy_probe.generated.rs");
    assert!(probe.contains(&prov.channels[0].schema_id));
    assert!(probe.contains("out[0..8].copy_from_slice(&self.yaw"));
    assert!(probe.contains("out[8..16].copy_from_slice(&self.pitch"));
    let depth = read("vehicle_depth_reversed.generated.rs");
    assert!(depth.contains("out[0..8].copy_from_slice(&self.uncertainty"));
    let exp = expected();
    let records = exp["records"].as_array().unwrap();
    assert_eq!(rec.records().len(), records.len());
    for (r, e) in rec.records().iter().zip(records) {
        assert_eq!(u64::from(r.channel_id), e["channel_id"].as_u64().unwrap());
        assert_eq!(r.sequence, e["sequence"].as_u64().unwrap());
        assert_eq!(
            r.timestamp,
            Timestamp::new(
                domain(e["domain"].as_str().unwrap()),
                i128::from(e["nanos"].as_i64().unwrap())
            )
        );
    }
}

#[test]
fn l2_fixture_bytes_are_the_declaration_order_encoding() {
    let rec = fixture();
    let prov = provenance();
    let exp = expected();
    for (r, e) in rec.records().iter().zip(exp["records"].as_array().unwrap()) {
        match prov.channels.iter().find(|c| c.channel_id == r.channel_id) {
            Some(entry) => {
                let values = &e["values"];
                assert_eq!(r.payload, legacy_encode(&entry.field_order, values));
                let layout = LegacyLayout::new(entry).unwrap();
                let decoded = layout.decode(&r.payload).unwrap();
                let names: Vec<_> = decoded.iter().map(|(n, _)| n.as_str()).collect();
                let order: Vec<_> = entry.field_order.iter().map(|f| f.name.as_str()).collect();
                assert_eq!(names, order);
                for (name, value) in decoded {
                    let want = &values[&name];
                    match value {
                        LegacyScalar::F64(x) => {
                            assert_eq!(x.to_bits(), want.as_f64().unwrap().to_bits(), "{name}")
                        }
                        LegacyScalar::F32(x) => assert_eq!(x, want.as_f64().unwrap() as f32),
                        LegacyScalar::I32(x) => assert_eq!(i64::from(x), want.as_i64().unwrap()),
                        LegacyScalar::I64(x) => assert_eq!(x, want.as_i64().unwrap()),
                        LegacyScalar::U32(x) => assert_eq!(u64::from(x), want.as_u64().unwrap()),
                        LegacyScalar::U64(x) => assert_eq!(x, want.as_u64().unwrap()),
                        LegacyScalar::Bool(x) => assert_eq!(x, want.as_bool().unwrap()),
                    }
                }
            }
            None => assert_eq!(hex(&r.payload), e["opaque_hex"].as_str().unwrap()),
        }
    }
}

#[test]
fn l3_migration_converts_values_and_preserves_everything_else() {
    let source = fixture();
    let exp = expected();
    let migrated = migrate_legacy_scalar(&source, &provenance()).unwrap();
    for (r, e) in migrated
        .records()
        .iter()
        .zip(exp["records"].as_array().unwrap())
    {
        let original = source
            .records()
            .iter()
            .find(|o| o.channel_id == r.channel_id && o.sequence == r.sequence)
            .unwrap();
        assert_eq!(r.timestamp, original.timestamp);
        match e.get("values") {
            Some(values) => {
                let layout = layout_of(channel_contract(r.channel_id));
                assert_eq!(hex(&r.payload), hex(&v2_encode(&layout, values)));
            }
            None => assert_eq!(r.payload, original.payload, "opaque bytes changed"),
        }
    }
    // Order, channel ids and sequences are unchanged.
    let keys = |rec: &dyn Recording| -> Vec<(u16, u64)> {
        rec.records()
            .iter()
            .map(|r| (r.channel_id, r.sequence))
            .collect()
    };
    assert_eq!(keys(&migrated), keys(&source));

    let m = migrated.manifest();
    let s = source.manifest();
    assert_eq!(
        (m.format_version, &m.writer, m.seed, &m.note),
        (s.format_version, &s.writer, s.seed, &s.note)
    );
    assert_eq!(&m.software[..s.software.len()], &s.software[..]);
    let added = &m.software[s.software.len()];
    assert_eq!(added.name, MIGRATION_SOFTWARE);
    assert!(added.version.ends_with(LEGACY_PRODUCER_REVISION));
    for (mc, sc) in m.channels.iter().zip(&s.channels) {
        assert_eq!(
            (mc.id, &mc.name, &mc.schema_id, &mc.clock_domain),
            (sc.id, &sc.name, &sc.schema_id, &sc.clock_domain)
        );
    }
    assert_eq!(m.channel(2).unwrap().wire, None);
    for (id, spec) in exp["channels"].as_object().unwrap() {
        let id: u16 = id.parse().unwrap();
        let wire = m.channel(id).unwrap().wire.clone().unwrap();
        assert_eq!(wire.codec_id, "neuradix.scalar-le.v2");
        assert_eq!(wire.wire_id, spec["canonical_wire_id"].as_str().unwrap());
        assert_eq!(wire.wire_id, layout_of(channel_contract(id)).wire_id);
    }

    let report = migrated.report();
    assert_eq!(report.source_codec, LEGACY_CODEC_LABEL);
    assert_eq!(report.source_revision, LEGACY_PRODUCER_REVISION);
    assert_eq!(report.source_digest, replay_digest(&source));
    assert_eq!(report.source_digest, exp["source_digest"].as_str().unwrap());
    assert_eq!(report.migrated_digest, replay_digest(&migrated));
    assert_eq!(
        report.migrated_digest,
        exp["migrated_digest"].as_str().unwrap()
    );
    assert_ne!(report.source_digest, report.migrated_digest);
    let counts: Vec<_> = report
        .channels
        .iter()
        .map(|c| (c.channel_id, c.records, c.legacy_wire_len))
        .collect();
    assert_eq!(counts, [(1, 3, 45), (3, 2, 16)]);

    // The migrated recording round-trips through the native container.
    let bytes = migrated.to_native_bytes().unwrap();
    let reread = NativeRecording::from_bytes(&bytes).unwrap();
    assert_eq!(reread.manifest(), migrated.manifest());
    assert_eq!(reread.records(), migrated.records());
    // ...and through the historical Neuradix MCAP profile, wire metadata
    // included. MCAP times are unsigned, so the one negative-time opaque
    // record (native-only, by design) is left out of this leg.
    let bytes = mcap_bytes(migrated.manifest(), migrated.records());
    let mcap = McapRecording::from_bytes(&bytes).unwrap();
    assert_eq!(mcap.manifest(), migrated.manifest());
    assert_eq!(mcap.records(), mcap_representable(migrated.records()));
}

#[test]
fn l4_reading_legacy_bytes_as_v2_silently_swaps_equal_width_fields() {
    // Channel 3: same schema identity, same 16-byte length, as the current
    // vehicle-depth contract. Reinterpreting instead of migrating is wrong.
    let source = fixture();
    let layout = layout_of("depth-reversed.yaml");
    let exp = expected();
    let depth_records: Vec<_> = source.records_for(3).collect();
    let values: Vec<_> = exp["records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["channel_id"] == 3)
        .map(|e| &e["values"])
        .collect();
    let field = |payload: &[u8], name: &str| {
        let f = layout.fields.iter().find(|f| f.name == name).unwrap();
        f64::from_le_bytes(payload[f.offset..f.offset + 8].try_into().unwrap())
    };
    let migrated = migrate_legacy_scalar(&source, &provenance()).unwrap();
    let converted: Vec<_> = migrated.records_for(3).collect();
    for ((legacy, v2), want) in depth_records.iter().zip(&converted).zip(&values) {
        assert_eq!(legacy.payload.len(), layout.wire_len);
        // Naive v2 interpretation: plausible numbers, wrong fields.
        assert_eq!(
            field(&legacy.payload, "depth"),
            want["uncertainty"].as_f64().unwrap()
        );
        assert_eq!(
            field(&legacy.payload, "uncertainty"),
            want["depth"].as_f64().unwrap()
        );
        // Explicit migration: correct fields.
        assert_eq!(field(&v2.payload, "depth"), want["depth"].as_f64().unwrap());
        assert_eq!(
            field(&v2.payload, "uncertainty"),
            want["uncertainty"].as_f64().unwrap()
        );
    }
}

fn rejects(prov: &LegacyProvenance) -> MigrationError {
    migrate_legacy_scalar(&fixture(), prov).unwrap_err()
}

#[test]
fn l5_missing_inconsistent_or_unsupported_provenance_is_rejected() {
    let base = provenance();
    let edit = |f: &dyn Fn(&mut LegacyProvenance)| {
        let mut p = base.clone();
        f(&mut p);
        rejects(&p)
    };
    assert!(matches!(
        edit(&|p| p.provenance_version = "neuradix.legacy-scalar-provenance.v0".into()),
        MigrationError::UnsupportedProvenanceVersion(_)
    ));
    for rev in [
        "c8aa467",
        "7a30679",
        "",
        &LEGACY_PRODUCER_REVISION.to_uppercase(),
    ] {
        let r = rev.to_owned();
        assert!(matches!(
            edit(&move |p| p.producer.revision = r.clone()),
            MigrationError::UnsupportedRevision(_)
        ));
    }
    for generator in ["cpp", "rust", "golden"] {
        let g = generator.to_owned();
        assert!(matches!(
            edit(&move |p| p.producer.generator = g.clone()),
            MigrationError::UnsupportedGenerator(_)
        ));
    }
    assert_eq!(edit(&|p| p.channels.clear()), MigrationError::NoChannels);
    assert_eq!(
        edit(&|p| {
            let c = p.channels[0].clone();
            p.channels.push(c)
        }),
        MigrationError::DuplicateChannel(1)
    );
    assert_eq!(
        edit(&|p| p.channels[0].channel_id = 9),
        MigrationError::UnknownChannel(9)
    );
    // Opaque channel 2 cannot be declared legacy: its schema does not match.
    assert!(matches!(
        edit(&|p| p.channels[0].channel_id = 2),
        MigrationError::SchemaMismatch { channel: 2, .. }
    ));
    // Channel 3's contract on channel 1: provenance and recording disagree.
    assert!(matches!(
        edit(&|p| {
            let depth = p.channels[1].clone();
            p.channels[0] = LegacyChannelProvenanceExt::with_id(depth, 1);
            p.channels.truncate(1);
        }),
        MigrationError::SchemaMismatch { channel: 1, .. }
    ));
    // Contract source edited: it no longer produces the recorded identity.
    assert!(matches!(
        edit(&|p| p.channels[0].contract = p.channels[0].contract.replace("unit: Hz", "unit: kHz")),
        MigrationError::SchemaMismatch { channel: 1, .. }
    ));
    assert!(matches!(
        edit(&|p| p.channels[0].contract = "apiVersion: nope\n".into()),
        MigrationError::Contract { channel: 1, .. }
    ));
    // Declared order is not the authored order (the silent-swap hazard).
    assert_eq!(
        edit(&|p| p.channels[0].field_order.swap(0, 1)),
        MigrationError::FieldOrderMismatch { channel: 1 }
    );
    assert_eq!(
        edit(&|p| p.channels[1].field_order.swap(0, 1)),
        MigrationError::FieldOrderMismatch { channel: 3 }
    );
    assert_eq!(
        edit(&|p| {
            p.channels[0].field_order.pop();
        }),
        MigrationError::FieldOrderMismatch { channel: 1 }
    );
    assert_eq!(
        edit(&|p| p.channels[0].field_order[4].ty = "float64".into()),
        MigrationError::FieldOrderMismatch { channel: 1 }
    );
    assert!(matches!(
        edit(&|p| p.channels[0].contract = "#".repeat(MAX_CONTRACT_SOURCE_BYTES + 1)),
        MigrationError::ContractTooLarge { channel: 1, .. }
    ));

    // Document-level: unknown or missing fields, oversize.
    let json = read("provenance.json");
    let mut v: Value = serde_json::from_str(&json).unwrap();
    v["producer"]["target"] = "avr".into();
    assert!(matches!(
        LegacyProvenance::from_json(v.to_string().as_bytes()),
        Err(MigrationError::Provenance(_))
    ));
    let mut v: Value = serde_json::from_str(&json).unwrap();
    v["channels"][0]
        .as_object_mut()
        .unwrap()
        .remove("field_order");
    assert!(matches!(
        LegacyProvenance::from_json(v.to_string().as_bytes()),
        Err(MigrationError::Provenance(_))
    ));
    let mut v: Value = serde_json::from_str(&json).unwrap();
    v.as_object_mut().unwrap().remove("producer");
    assert!(matches!(
        LegacyProvenance::from_json(v.to_string().as_bytes()),
        Err(MigrationError::Provenance(_))
    ));
    assert!(matches!(
        LegacyProvenance::from_json(&vec![b' '; MAX_PROVENANCE_BYTES + 1]),
        Err(MigrationError::ProvenanceTooLarge(_))
    ));
}

/// Test helper: rebind a provenance entry to another channel id.
trait LegacyChannelProvenanceExt {
    fn with_id(self, id: u16) -> Self;
}
impl LegacyChannelProvenanceExt for neuradix_record::legacy_scalar::LegacyChannelProvenance {
    fn with_id(mut self, id: u16) -> Self {
        self.channel_id = id;
        self
    }
}

#[test]
fn l6_unsupported_fields_and_already_bound_channels_are_rejected() {
    let mut entry = provenance().channels[0].clone();
    entry.contract = entry.contract.replace(
        "      flags:\n        type: uint32",
        "      label:\n        type: string",
    );
    entry.field_order[7] = LegacyField {
        name: "label".into(),
        ty: "string".into(),
    };
    let schema = schema_identity(
        &neuradix_contracts::validate::from_yaml_str(&entry.contract, Path::new("<t>")).unwrap(),
    );
    entry.schema_id = schema.as_str().into();
    assert!(matches!(
        LegacyLayout::new(&entry),
        Err(MigrationError::UnsupportedField { channel: 1, .. })
    ));

    // A channel that already records a wire binding is not a legacy stream.
    let source = fixture();
    let mut manifest = source.manifest().clone();
    manifest.channels[0].wire = Some(ChannelWire {
        codec_id: "neuradix.scalar-le.v2".into(),
        wire_id: layout_of("contract.yaml").wire_id,
    });
    let bound = rewrite(&manifest, source.records());
    assert_eq!(
        migrate_legacy_scalar(&bound, &provenance()).unwrap_err(),
        MigrationError::AlreadyBound(1)
    );
}

fn rewrite(manifest: &RecordingManifest, records: &[RawRecord]) -> NativeRecording {
    let mut w = NativeRecordWriter::new(Vec::new(), manifest).unwrap();
    for r in records {
        w.write_record(r.channel_id, r.sequence, r.timestamp, &r.payload)
            .unwrap();
    }
    NativeRecording::from_bytes(&w.finish().unwrap()).unwrap()
}

#[test]
fn l7_malformed_payloads_reject_the_whole_request() {
    let source = fixture();
    let index = source
        .records()
        .iter()
        .position(|r| r.channel_id == 1)
        .unwrap();
    type Mutation<'a> = &'a dyn Fn(&mut Vec<u8>);
    let cases: [(Mutation, PayloadError); 4] = [
        (
            &|p| {
                p.pop();
            },
            PayloadError::Length {
                expected: 45,
                actual: 44,
            },
        ),
        (
            &|p| p.push(0),
            PayloadError::Length {
                expected: 45,
                actual: 46,
            },
        ),
        (
            &|p| p.clear(),
            PayloadError::Length {
                expected: 45,
                actual: 0,
            },
        ),
        (
            &|p| p[20] = 2,
            PayloadError::NonCanonicalBool {
                field: "armed".into(),
                byte: 2,
            },
        ),
    ];
    for (mutate, error) in cases {
        let mut records = source.records().to_vec();
        mutate(&mut records[index].payload);
        let bad = rewrite(source.manifest(), &records);
        assert_eq!(
            migrate_legacy_scalar(&bad, &provenance()).unwrap_err(),
            MigrationError::Payload {
                channel: 1,
                sequence: 10,
                index,
                error,
            }
        );
    }
}

#[test]
fn l8_channels_without_provenance_stay_opaque() {
    // Only channel 3 is declared legacy. Channel 1 has the same container
    // version and no wire metadata, but is never reinterpreted.
    let source = fixture();
    let mut prov = provenance();
    prov.channels.remove(0);
    let migrated = migrate_legacy_scalar(&source, &prov).unwrap();
    assert!(migrated.manifest().channel(1).unwrap().wire.is_none());
    let before: Vec<_> = source.records_for(1).map(|r| r.payload.clone()).collect();
    let after: Vec<_> = migrated.records_for(1).map(|r| r.payload.clone()).collect();
    assert_eq!(before, after);
    assert_eq!(migrated.report().channels.len(), 1);
}

#[test]
fn l9_wire_metadata_is_optional_and_compatible() {
    let source = fixture();
    // Manifests without wire metadata serialize without a `wire` key, as before.
    let json = serde_json::to_string(source.manifest()).unwrap();
    assert!(!json.contains("\"wire\""));
    // The fixture's native bytes re-encode identically with the current writer.
    assert_eq!(
        rewrite(source.manifest(), source.records()).manifest(),
        source.manifest()
    );
    let reencoded = {
        let mut w = NativeRecordWriter::new(Vec::new(), source.manifest()).unwrap();
        for r in source.records() {
            w.write_record(r.channel_id, r.sequence, r.timestamp, &r.payload)
                .unwrap();
        }
        w.finish().unwrap()
    };
    assert_eq!(reencoded, fixture_bytes());
    // The replay digest excludes the manifest: wire metadata alone changes nothing.
    let mut manifest = source.manifest().clone();
    manifest.channels[1].wire = Some(ChannelWire {
        codec_id: "opaque".into(),
        wire_id: "sha256:".to_owned() + &"0".repeat(64),
    });
    assert_eq!(
        replay_digest(&rewrite(&manifest, source.records())),
        replay_digest(&source)
    );
    // Unknown fields inside `wire` are rejected.
    let mut v: Value = serde_json::to_value(&manifest).unwrap();
    v["channels"][1]["wire"]["extra"] = 1.into();
    assert!(serde_json::from_value::<RecordingManifest>(v).is_err());
}

#[test]
fn l10_mcap_profile_writes_and_checks_wire_metadata() {
    let migrated = migrate_legacy_scalar(&fixture(), &provenance()).unwrap();
    let bytes = mcap_bytes(migrated.manifest(), migrated.records());
    let archive = McapArchive::from_reader(&bytes[..], McapImportLimits::default()).unwrap();
    let meta = |id: u16| archive.summary().channels()[&id].metadata.clone();
    assert_eq!(meta(2).len(), 2, "no wire keys without wire metadata");
    let wire = migrated
        .manifest()
        .channel(1)
        .unwrap()
        .wire
        .clone()
        .unwrap();
    assert_eq!(
        meta(1),
        BTreeMap::from([
            ("clockDomain".into(), "monotonic".into()),
            ("codecId".into(), wire.codec_id.clone()),
            (
                "schemaIdentity".into(),
                migrated.manifest().channel(1).unwrap().schema_id.clone()
            ),
            ("wireId".into(), wire.wire_id.clone()),
        ])
    );

    // A historical-profile archive whose channel metadata disagrees with the
    // manifest's wire binding is not projected.
    let manifest = migrated.manifest();
    let build = |drop_wire_id: bool| {
        let mut w = McapStreamWriter::new(
            Vec::new(),
            &McapHeader {
                profile: "neuradix".into(),
                library: concat!("neuradix-record/", env!("CARGO_PKG_VERSION")).into(),
            },
            McapWriteLimits::default(),
        )
        .unwrap();
        w.metadata(&McapMetadata {
            name: "neuradix.manifest".into(),
            entries: BTreeMap::from([("json".into(), serde_json::to_string(manifest).unwrap())]),
        })
        .unwrap();
        for (i, c) in manifest.channels.iter().enumerate() {
            let schema = i as u16 + 1;
            w.schema(&McapSchema {
                id: schema,
                name: c.name.clone(),
                encoding: "neuradix/schema-id".into(),
                data: c.schema_id.as_bytes().to_vec(),
            })
            .unwrap();
            let mut metadata = BTreeMap::from([
                ("clockDomain".into(), c.clock_domain.clone()),
                ("schemaIdentity".into(), c.schema_id.clone()),
            ]);
            if let Some(wire) = &c.wire {
                metadata.insert("codecId".into(), wire.codec_id.clone());
                if !drop_wire_id {
                    metadata.insert("wireId".into(), wire.wire_id.clone());
                }
            }
            w.channel(&McapChannel {
                id: c.id,
                schema_id: schema,
                topic: c.name.clone(),
                message_encoding: "neuradix".into(),
                metadata,
            })
            .unwrap();
        }
        w.finish().unwrap()
    };
    assert!(McapRecording::from_bytes(&build(false)).is_ok());
    assert!(matches!(
        McapRecording::from_bytes(&build(true)),
        Err(RecordError::UnsupportedMcap(_))
    ));
}

fn mcap_representable(records: &[RawRecord]) -> Vec<RawRecord> {
    records
        .iter()
        .filter(|r| r.timestamp.as_nanos() >= 0)
        .cloned()
        .collect()
}

fn mcap_bytes(manifest: &RecordingManifest, records: &[RawRecord]) -> Vec<u8> {
    let mut writer = McapWriter::new(Vec::new(), manifest).unwrap();
    for r in &mcap_representable(records) {
        writer
            .write_record(r.channel_id, r.sequence, r.timestamp, &r.payload)
            .unwrap();
    }
    writer.finish().unwrap()
}
