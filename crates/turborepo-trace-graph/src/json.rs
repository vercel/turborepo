//! The versioned JSON graph format (see `SCHEMA.md`): export from any
//! [`GraphView`], import into a [`MemGraph`].

use std::{borrow::Cow, fmt, io::Write, marker::PhantomData};

use serde::{
    Deserialize, Serialize,
    de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor},
    ser::{SerializeStruct, Serializer},
};

use crate::{
    mem::{BuildError, GraphBuilder, MemGraph},
    view::{GraphView, Membership, NodeId, PackageId, PackageVersion, Span},
    vocab::{EdgeAttr, Effect, Origin, Phase, RefFlags, RefKind, UncertaintyKind},
};

/// The `schema` field value.
pub const SCHEMA_NAME: &str = "turbotrace-graph";
/// The current `version` field value.
pub const SCHEMA_VERSION: u32 = 1;

/// Errors loading a JSON graph.
#[derive(Debug, thiserror::Error)]
pub enum JsonError {
    /// Malformed JSON or a field of the wrong type.
    #[error("invalid graph JSON: {0}")]
    Syntax(#[from] serde_json::Error),
    /// Not a `turbo trace` graph, or an unsupported version.
    #[error("{0}")]
    Schema(String),
    /// Dangling references or duplicate paths.
    #[error("invalid graph: {0}")]
    Build(#[from] BuildError),
}

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum Field {
    Schema,
    Version,
    Root,
    Packages,
    Files,
    Edges,
    Uncertainties,
    Probes,
    Memberships,
}

#[derive(Deserialize)]
struct InPackage<'a> {
    #[serde(borrow)]
    name: Cow<'a, str>,
    #[serde(borrow)]
    version: Cow<'a, str>,
    #[serde(default, borrow)]
    root: Cow<'a, str>,
}

#[derive(Deserialize)]
struct InFile<'a> {
    #[serde(borrow)]
    path: Cow<'a, str>,
    package: u32,
    #[serde(default, borrow)]
    lang: Option<Cow<'a, str>>,
    #[serde(default)]
    external: bool,
}

#[derive(Deserialize)]
struct InEdge<'a> {
    from: u32,
    #[serde(default)]
    to: Option<u32>,
    kind: RefKind,
    #[serde(default)]
    phase: Option<Phase>,
    #[serde(default)]
    flags: RefFlags,
    #[serde(default)]
    line: u32,
    #[serde(default)]
    span: Option<[u32; 2]>,
    #[serde(default, borrow)]
    specifier: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct InUncertainty<'a> {
    file: u32,
    kind: UncertaintyKind,
    #[serde(default)]
    line: u32,
    #[serde(default, borrow)]
    detail: Cow<'a, str>,
    #[serde(default, borrow)]
    scope: Option<InScope<'a>>,
    #[serde(default)]
    effect: Option<Effect>,
    #[serde(default)]
    origin: Option<Origin>,
}

#[derive(Deserialize)]
struct InScope<'a> {
    #[serde(borrow)]
    dir: Cow<'a, str>,
    #[serde(borrow)]
    pattern: Cow<'a, str>,
}

#[derive(Deserialize)]
struct InProbe<'a> {
    file: u32,
    #[serde(borrow)]
    path: Cow<'a, str>,
}

#[derive(Deserialize)]
struct InMembership<'a> {
    file: u32,
    #[serde(borrow)]
    dir: Cow<'a, str>,
    #[serde(borrow)]
    pattern: Cow<'a, str>,
}

/// Deserializes a JSON array element by element, handing each to `f` (no
/// intermediate `Vec`).
struct EachSeed<T, F>(F, PhantomData<fn() -> T>);

impl<'de, T: Deserialize<'de>, F: FnMut(T)> DeserializeSeed<'de> for EachSeed<T, F> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}

impl<'de, T: Deserialize<'de>, F: FnMut(T)> Visitor<'de> for EachSeed<T, F> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array")
    }
    fn visit_seq<A: SeqAccess<'de>>(mut self, mut seq: A) -> Result<(), A::Error> {
        while let Some(x) = seq.next_element::<T>()? {
            (self.0)(x);
        }
        Ok(())
    }
}

fn each<'de, T: Deserialize<'de>, F: FnMut(T)>(f: F) -> EachSeed<T, F> {
    EachSeed(f, PhantomData)
}

/// Streams the top-level object into a builder; returns `(schema, version)`.
struct GraphVisitor<'b>(&'b mut GraphBuilder);

impl<'de> Visitor<'de> for GraphVisitor<'_> {
    type Value = (Option<String>, Option<u32>);

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a turbo trace graph object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let b = self.0;
        let (mut schema, mut version) = (None, None);
        while let Some(key) = map.next_key::<Field>()? {
            match key {
                Field::Schema => schema = Some(map.next_value::<String>()?),
                Field::Version => version = Some(map.next_value::<u32>()?),
                Field::Root => {
                    if let Some(r) = map.next_value::<Option<String>>()? {
                        b.root(r);
                    }
                }
                Field::Packages => map.next_value_seed(each(|p: InPackage<'de>| {
                    let v = (p.version != "workspace").then_some(&*p.version);
                    b.package(&p.name, v, &p.root);
                }))?,
                Field::Files => map.next_value_seed(each(|f: InFile<'de>| {
                    b.add_file(&f.path, PackageId(f.package), f.lang.as_deref(), f.external);
                }))?,
                Field::Edges => map.next_value_seed(each(|e: InEdge<'de>| {
                    let phase = e.phase.unwrap_or(e.kind.default_phase());
                    b.add_edge_raw(
                        NodeId(e.from),
                        e.to.map(NodeId),
                        EdgeAttr::new(e.kind, phase, e.flags),
                        e.line,
                        e.span.map(|[start, end]| Span { start, end }),
                        e.specifier.as_deref(),
                    );
                }))?,
                Field::Uncertainties => map.next_value_seed(each(|u: InUncertainty<'de>| {
                    match &u.scope {
                        Some(sc) => b.covered_uncertainty(
                            NodeId(u.file),
                            u.kind,
                            u.line,
                            &u.detail,
                            &sc.dir,
                            &sc.pattern,
                        ),
                        None => b.uncertainty(NodeId(u.file), u.kind, u.line, &u.detail),
                    };
                    b.uncertainty_class(
                        u.effect.unwrap_or(Effect::None),
                        u.origin.unwrap_or(Origin::Unknown),
                    );
                }))?,
                Field::Probes => map.next_value_seed(each(|p: InProbe<'de>| {
                    b.probe(NodeId(p.file), &p.path);
                }))?,
                Field::Memberships => map.next_value_seed(each(|m: InMembership<'de>| {
                    b.membership(NodeId(m.file), &m.dir, &m.pattern);
                }))?,
            }
        }
        Ok((schema, version))
    }
}

impl MemGraph {
    /// Loads a graph from JSON bytes. Records are streamed straight into the
    /// builder; references are validated when the graph is frozen.
    pub fn from_json_slice(bytes: &[u8]) -> Result<MemGraph, JsonError> {
        #[derive(Deserialize)]
        struct Header {
            schema: Option<String>,
            version: Option<u32>,
        }
        let check = |schema: Option<&str>, version: Option<u32>| {
            if schema != Some(SCHEMA_NAME) {
                return Err(JsonError::Schema(format!(
                    "not a turbo trace graph (expected \"schema\": \"{SCHEMA_NAME}\")"
                )));
            }
            if version != Some(SCHEMA_VERSION) {
                return Err(JsonError::Schema(format!(
                    "unsupported graph schema version {} (this build reads version \
                     {SCHEMA_VERSION})",
                    version.map_or("(missing)".to_owned(), |v| v.to_string())
                )));
            }
            Ok(())
        };
        let mut b = MemGraph::builder();
        let mut de = serde_json::Deserializer::from_slice(bytes);
        let parsed = de
            .deserialize_map(GraphVisitor(&mut b))
            .and_then(|h| de.end().map(|()| h));
        match parsed {
            Ok((schema, version)) => {
                check(schema.as_deref(), version)?;
                Ok(b.build()?)
            }
            Err(e) => {
                // Prefer a schema error for foreign or future files over a field error.
                if let Ok(h) = serde_json::from_slice::<Header>(bytes) {
                    check(h.schema.as_deref(), h.version)?;
                }
                Err(e.into())
            }
        }
    }

    /// Loads a graph from a JSON string.
    pub fn from_json_str(s: &str) -> Result<MemGraph, JsonError> {
        Self::from_json_slice(s.as_bytes())
    }

    /// Serializes this graph (see [`write_json`]).
    // Invariant: writing to a Vec cannot fail; the JSON writer emits UTF-8.
    #[allow(clippy::expect_used)]
    pub fn to_json_string(&self) -> String {
        let mut out = Vec::new();
        write_json(self, self.root(), false, &mut out).expect("writing to a Vec");
        String::from_utf8(out).expect("JSON is UTF-8")
    }
}

struct Seq<F>(F);

impl<F, I, T> Serialize for Seq<F>
where
    F: Fn() -> I,
    I: Iterator<Item = T>,
    T: Serialize,
{
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq((self.0)())
    }
}

#[derive(Serialize)]
struct OutPackage<'a> {
    name: &'a str,
    version: &'a str,
    root: &'a str,
}

#[derive(Serialize)]
struct OutFile<'a> {
    path: &'a str,
    package: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    external: bool,
}

#[derive(Serialize)]
struct OutEdge<'a> {
    from: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<u32>,
    kind: RefKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    phase: Option<Phase>,
    #[serde(skip_serializing_if = "no_flags")]
    flags: RefFlags,
    #[serde(skip_serializing_if = "is_zero")]
    line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    span: Option<[u32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    specifier: Option<&'a str>,
}

fn is_zero(x: &u32) -> bool {
    *x == 0
}

fn no_flags(f: &RefFlags) -> bool {
    f.is_empty()
}

#[derive(Serialize)]
struct OutUncertainty<'a> {
    file: u32,
    kind: UncertaintyKind,
    #[serde(skip_serializing_if = "is_zero")]
    line: u32,
    detail: &'a str,
    /// Present for a covered uncertainty.
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<OutScope<'a>>,
    /// Present when the record was raised by a reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    effect: Option<Effect>,
    /// Present when the origin of the unknown value is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<Origin>,
}

fn out_effect(e: Effect) -> Option<Effect> {
    (e != Effect::None).then_some(e)
}

fn out_origin(o: Origin) -> Option<Origin> {
    (o != Origin::Unknown).then_some(o)
}

#[derive(Serialize)]
struct OutScope<'a> {
    dir: &'a str,
    pattern: &'a str,
}

fn out_scope(m: Option<Membership<'_>>) -> Option<OutScope<'_>> {
    m.map(|m| OutScope {
        dir: m.dir,
        pattern: m.pattern,
    })
}

#[derive(Serialize)]
struct OutProbe<'a> {
    file: u32,
    path: &'a str,
}

#[derive(Serialize)]
struct OutMembership<'a> {
    file: u32,
    dir: &'a str,
    pattern: &'a str,
}

struct Export<'g, G> {
    g: &'g G,
    root: Option<&'g str>,
}

impl<G: GraphView> Serialize for Export<'_, G> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let g = self.g;
        let nodes = move || (0..g.node_count()).map(NodeId);
        let mut st = s.serialize_struct("Graph", 9)?;
        st.serialize_field("schema", SCHEMA_NAME)?;
        st.serialize_field("version", &SCHEMA_VERSION)?;
        if let Some(r) = self.root {
            st.serialize_field("root", r)?;
        }
        st.serialize_field(
            "packages",
            &Seq(|| {
                (0..g.package_count()).map(|p| {
                    let i = g.package_info(PackageId(p));
                    OutPackage {
                        name: i.name,
                        version: match i.version {
                            PackageVersion::Workspace => "workspace",
                            PackageVersion::Version(v) => v,
                        },
                        root: i.root,
                    }
                })
            }),
        )?;
        st.serialize_field(
            "files",
            &Seq(|| {
                nodes().map(|n| OutFile {
                    path: g.path(n),
                    package: g.package(n).0,
                    lang: g.lang(n),
                    external: g.is_external(n),
                })
            }),
        )?;
        st.serialize_field(
            "edges",
            &Seq(|| {
                nodes().flat_map(move |n| {
                    g.out_edges(n).map(move |e| OutEdge {
                        from: n.0,
                        to: e.to.map(|t| t.0),
                        kind: e.kind(),
                        phase: (e.phase() != e.kind().default_phase()).then_some(e.phase()),
                        flags: e.flags(),
                        line: e.line,
                        span: e.span.map(|s| [s.start, s.end]),
                        specifier: e.specifier,
                    })
                })
            }),
        )?;
        st.serialize_field(
            "uncertainties",
            &Seq(|| {
                nodes().flat_map(move |n| {
                    g.uncertainties(n).map(move |u| OutUncertainty {
                        file: n.0,
                        kind: u.kind,
                        line: u.line,
                        detail: u.detail,
                        scope: out_scope(u.scope),
                        effect: out_effect(u.effect),
                        origin: out_origin(u.origin),
                    })
                })
            }),
        )?;
        st.serialize_field(
            "probes",
            &Seq(|| {
                nodes().flat_map(move |n| g.probes(n).map(move |p| OutProbe { file: n.0, path: p }))
            }),
        )?;
        st.serialize_field(
            "memberships",
            &Seq(|| {
                nodes().flat_map(move |n| {
                    g.memberships(n).map(move |m| OutMembership {
                        file: n.0,
                        dir: m.dir,
                        pattern: m.pattern,
                    })
                })
            }),
        )?;
        st.end()
    }
}

#[derive(Serialize)]
struct Tagged<T> {
    #[serde(rename = "type")]
    ty: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u32>,
    #[serde(flatten)]
    rec: T,
}

/// Writes any graph as newline-delimited JSON (`SCHEMA.md`, "NDJSON"): a header
/// line, then one record per line, tagged with `"type"`, in the order packages,
/// files, edges, uncertainties, probes, memberships. Records carry the same
/// fields as in the JSON format; packages and files also carry their index as
/// `"id"`.
pub fn write_ndjson<G: GraphView>(
    g: &G,
    root: Option<&str>,
    w: &mut dyn Write,
) -> std::io::Result<()> {
    let mut w = std::io::BufWriter::new(w);
    #[derive(Serialize)]
    struct Header<'a> {
        schema: &'static str,
        version: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        root: Option<&'a str>,
    }
    let mut line = |v: &dyn erased::Ser| -> std::io::Result<()> {
        v.write(&mut w)?;
        w.write_all(b"\n")
    };
    line(&Header {
        schema: "turbotrace-graph-ndjson",
        version: SCHEMA_VERSION,
        root,
    })?;
    for p in 0..g.package_count() {
        let i = g.package_info(PackageId(p));
        line(&Tagged {
            ty: "package",
            id: Some(p),
            rec: OutPackage {
                name: i.name,
                version: i.version.as_str(),
                root: i.root,
            },
        })?;
    }
    let nodes = || (0..g.node_count()).map(NodeId);
    for n in nodes() {
        line(&Tagged {
            ty: "file",
            id: Some(n.0),
            rec: OutFile {
                path: g.path(n),
                package: g.package(n).0,
                lang: g.lang(n),
                external: g.is_external(n),
            },
        })?;
    }
    for n in nodes() {
        for e in g.out_edges(n) {
            line(&Tagged {
                ty: "edge",
                id: None,
                rec: OutEdge {
                    from: n.0,
                    to: e.to.map(|t| t.0),
                    kind: e.kind(),
                    phase: (e.phase() != e.kind().default_phase()).then_some(e.phase()),
                    flags: e.flags(),
                    line: e.line,
                    span: e.span.map(|s| [s.start, s.end]),
                    specifier: e.specifier,
                },
            })?;
        }
    }
    for n in nodes() {
        for u in g.uncertainties(n) {
            line(&Tagged {
                ty: "uncertainty",
                id: None,
                rec: OutUncertainty {
                    file: n.0,
                    kind: u.kind,
                    line: u.line,
                    detail: u.detail,
                    scope: out_scope(u.scope),
                    effect: out_effect(u.effect),
                    origin: out_origin(u.origin),
                },
            })?;
        }
    }
    for n in nodes() {
        for p in g.probes(n) {
            line(&Tagged {
                ty: "probe",
                id: None,
                rec: OutProbe { file: n.0, path: p },
            })?;
        }
    }
    for n in nodes() {
        for m in g.memberships(n) {
            line(&Tagged {
                ty: "membership",
                id: None,
                rec: OutMembership {
                    file: n.0,
                    dir: m.dir,
                    pattern: m.pattern,
                },
            })?;
        }
    }
    w.flush()
}

/// Object-safe serialization to a writer, so `write_ndjson` can take mixed
/// record types.
mod erased {
    pub(super) trait Ser {
        fn write(&self, w: &mut dyn std::io::Write) -> std::io::Result<()>;
    }
    impl<T: serde::Serialize> Ser for T {
        fn write(&self, w: &mut dyn std::io::Write) -> std::io::Result<()> {
            serde_json::to_writer(w, self).map_err(std::io::Error::from)
        }
    }
}

/// Writes any graph in the JSON format (`pretty` for indented output).
pub fn write_json<G: GraphView>(
    g: &G,
    root: Option<&str>,
    pretty: bool,
    w: &mut dyn Write,
) -> std::io::Result<()> {
    let mut w = std::io::BufWriter::new(w);
    let export = Export { g, root };
    if pretty {
        serde_json::to_writer_pretty(&mut w, &export)?;
    } else {
        serde_json::to_writer(&mut w, &export)?;
    }
    writeln!(w)?;
    w.flush()
}
