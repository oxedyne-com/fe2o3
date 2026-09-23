//! The Fleet page is gated on `dashboard.admin`, not `dashboard.view`: it shows
//! what every watched peer says about itself, which is whether an attack on any
//! of them is working, and that is not for a read-only login on one of them.
//!
//! Every field a body carries is drawn somewhere. A field no pane knows lands
//! under *Other fields* rather than being dropped, so a peer on a newer build --
//! one reporting the age of a stamp file, say -- is visible here before this page
//! has been taught what the field means.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::{
    admin::{
        AdminPrincipal,
        assets::{
            FLEET_JS,
            render_layout,
        },
        handler::{
            PATH_FLEET,
            extract_principal,
            json_escape,
            redirect_to_login,
        },
        state::AdminState,
    },
    fleet::{
        FleetPeer,
        ProbeSample,
        RowState,
        Tone,
        peer_state,
        tone,
        unix_secs,
    },
    health::{
        F_CONNS,
        F_DISK_IOPS,
        F_DISK_PCT,
        F_DROPPED_1M,
        F_GUARD_SELFTEST,
        F_LOAD1,
        F_MAIL_DOWN,
        F_MEM_PCT,
        F_PROBE_MS,
        F_R429_1M,
        F_SEALED,
        F_SEALED_DBS,
        F_SWAP_PCT,
        F_UPTIME_S,
        HealthBody,
        RES_CAP_KB,
        RES_CAP_PCT,
        RES_PROCS,
        RES_RSS_KB,
        resident_key,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    fields::{
        HeaderFieldValue,
        HeaderFields,
        HeaderName,
    },
    msg::HttpMessage,
    status::HttpStatus,
};

use std::collections::{
    BTreeMap,
    BTreeSet,
};

/// One cell a pane always or sometimes shows.
struct CellSpec {
    key:    &'static str,
    label:  &'static str,
    unit:   &'static str,   // how the page formats the value
    always: bool,           // shown even when the body lacks the field
}

// Pane A, the box short of something. Residents follow these, one cell each.
const PANE_A: &[CellSpec] = &[
    CellSpec { key: F_MEM_PCT,          label: "Memory",        unit: "pct",    always: true },
    CellSpec { key: F_SWAP_PCT,         label: "Swap",          unit: "pct",    always: true },
    CellSpec { key: F_DISK_IOPS,        label: "Disk ops/s",    unit: "n",      always: true },
    CellSpec { key: F_LOAD1,            label: "Load",          unit: "load",   always: true },
    CellSpec { key: F_DISK_PCT,         label: "Disk space",    unit: "pct",    always: true },
];

// Pane B, someone asking a lot.
const PANE_B: &[CellSpec] = &[
    CellSpec { key: F_CONNS,            label: "Connections",   unit: "n",      always: true },
    CellSpec { key: F_R429_1M,          label: "429s / min",    unit: "n",      always: true },
    CellSpec { key: F_DROPPED_1M,       label: "Dropped / min", unit: "n",      always: true },
    CellSpec { key: F_GUARD_SELFTEST,   label: "Guard",         unit: "guard",  always: true },
    CellSpec { key: F_PROBE_MS,         label: "Probe",         unit: "ms",     always: true },
    CellSpec { key: F_MAIL_DOWN,        label: "Mail",          unit: "mail",   always: false },
    // Carries the raw `sealed` beside it, so it accounts for both fields.
    CellSpec { key: F_SEALED_DBS,       label: "Seal",          unit: "seal",   always: false },
];

// Read into the row head rather than a cell.
const HEAD_KEYS: &[&str] = &[F_UPTIME_S];

// The resident figures its cell draws; any other `res.` figure is shown as Other.
const RESIDENT_FIGURES: &[&str] = &[RES_PROCS, RES_RSS_KB, RES_CAP_KB, RES_CAP_PCT];

/// How the page formats a field nothing names, read from its suffix.
fn unit_of(key: &str) -> &'static str {
    if key.ends_with("_pct") {
        "pct"
    } else if key.ends_with("_ms") {
        "ms"
    } else if key.ends_with("_s") || key.ends_with("_secs") {
        "secs"
    } else if key.ends_with("_kb") {
        "kib"
    } else {
        "n"
    }
}

/// Who is asking, and whether they may see the page.
enum Gate {
    Admin(AdminPrincipal),
    NotAdmin(AdminPrincipal),
    SignedOut,
}

fn gate(state: &AdminState, headers: &HeaderFields) -> Gate {
    match extract_principal(state, headers) {
        None                                => Gate::SignedOut,
        Some(p) if p.can_admin_dashboard()  => Gate::Admin(p),
        Some(p)                             => Gate::NotAdmin(p),
    }
}

pub fn render_fleet_page(state: &AdminState, headers: &HeaderFields) -> HttpMessage {
    let principal = match gate(state, headers) {
        Gate::Admin(p)      => p,
        Gate::SignedOut     => return redirect_to_login(),
        Gate::NotAdmin(p)   => {
            let body = "<h1>Fleet</h1>\n\
                <p class=\"notice error\">The Fleet view needs the \
                <code>dashboard.admin</code> scope. It shows what every watched \
                host reports about itself, including whether an attack on it is \
                working, so a view-only sign-in does not reach it.</p>\n";
            let html = render_layout("Fleet", PATH_FLEET, &p, body, "");
            return html_with_status(HttpStatus::Forbidden, html);
        },
    };
    // A `<` can only sit inside a JSON string here, where `\u003c` means the same
    // thing and cannot end, or restart, the script element the data rides in.
    let data = fleet_json(state).replace('<', "\\u003c");
    let body = fmt!(
        "<h1>Fleet</h1>\n\
        <p class=\"meta\">What this host's watcher last read from each machine it \
        watches, beside this host's own reading. Each cell is judged against that \
        peer's own <code>distress</code> and <code>clear</code> thresholds, the \
        numbers its alarm uses: red at distress, amber between the two, green at or \
        under clear. A figure with no threshold is left uncoloured. Nothing on this \
        page raises, silences or acknowledges an alarm.</p>\n\
        <div id=\"fleet-notice\"></div>\n\
        <div id=\"fleet-rows\" class=\"fleet-rows\">\
            <p class=\"notice empty\">Drawing the fleet&hellip;</p></div>\n\
        <p class=\"meta fleet-stamp\" id=\"fleet-stamp\"></p>\n\
        <script id=\"fleet-data\" type=\"application/json\">{data}</script>\n\
        <script>{js}</script>\n",
        data = data,
        js   = FLEET_JS,
    );
    let html = render_layout("Fleet", PATH_FLEET, &principal, &body, "");
    html_with_status(HttpStatus::OK, html)
}

pub fn render_fleet_json(state: &AdminState, headers: &HeaderFields) -> HttpMessage {
    match gate(state, headers) {
        Gate::Admin(_)      => (),
        Gate::SignedOut     => return HttpMessage::respond_with_text(
            HttpStatus::Unauthorized, "Sign in required."),
        Gate::NotAdmin(_)   => return HttpMessage::respond_with_text(
            HttpStatus::Forbidden, "The Fleet view needs the dashboard.admin scope."),
    }
    HttpMessage::new_response(HttpStatus::OK)
        .with_field(
            HeaderName::ContentType,
            HeaderFieldValue::Generic("application/json; charset=utf-8".to_string()),
        )
        .with_field(
            HeaderName::CacheControl,
            HeaderFieldValue::Generic("no-store".to_string()),
        )
        .with_body(fleet_json(state).into_bytes())
}

fn html_with_status(status: HttpStatus, html: String) -> HttpMessage {
    HttpMessage::new_response(status)
        .with_field(
            HeaderName::ContentType,
            HeaderFieldValue::Generic("text/html; charset=utf-8".to_string()),
        )
        .with_field(
            HeaderName::CacheControl,
            HeaderFieldValue::Generic("no-store".to_string()),
        )
        .with_body(html.into_bytes())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE DOCUMENT                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// The whole page's data: this host's row first, then one row per watched host,
/// in the order the watch list first names each.
pub fn fleet_json(state: &AdminState) -> String {
    let fleet = &state.fleet;
    let now = unix_secs();
    let snap = match fleet.snapshot() {
        Ok(s) => s,
        Err(e) => {
            error!(e, "dashboard: fleet snapshot failed");
            Vec::new()
        },
    };
    let whoami = fleet.whoami();

    // Hosts in first-mention order, this host's own name kept for its row.
    let mut hosts: Vec<&str> = Vec::new();
    for (p, _) in &snap {
        if p.host != whoami && !hosts.contains(&p.host.as_str()) {
            hosts.push(p.host.as_str());
        }
    }

    let mut rows = Vec::with_capacity(hosts.len() + 1);
    let own_services: Vec<&(FleetPeer, Vec<ProbeSample>)> = snap.iter()
        .filter(|(p, _)| !whoami.is_empty() && p.host == whoami)
        .collect();
    rows.push(self_row_json(state, whoami, &own_services, now));
    for host in hosts {
        let group: Vec<&(FleetPeer, Vec<ProbeSample>)> = snap.iter()
            .filter(|(p, _)| p.host == host)
            .collect();
        rows.push(host_row_json(host, &group, now, fleet.interval_secs()));
    }

    fmt!(
        "{{\"now\":{now},\"whoami\":{who},\"watching\":{watching},\"link_down\":{link},\
        \"peers\":{peers},\"interval_secs\":{interval},\"fail_threshold\":{fail},\
        \"started\":{started},\"rows\":[{rows}]}}",
        now      = now,
        who      = jstr(whoami),
        watching = fleet.is_watching(),
        link     = fleet.is_link_down(),
        peers    = fleet.peers().len(),
        interval = fleet.interval_secs(),
        fail     = fleet.fail_threshold(),
        started  = fleet.started_secs(),
        rows     = rows.join(","),
    )
}

/// This host, from its own state: the reachability a self-probe would measure is
/// meaningless from the same box, so there is no probe time, and no threshold is
/// held here for this host -- its watchers hold those -- so nothing is coloured
/// but the guard's own self-test.
fn self_row_json(
    state:      &AdminState,
    whoami:     &str,
    services:   &[&(FleetPeer, Vec<ProbeSample>)],
    now:        u64,
)
    -> String
{
    let body = state.health_body();
    let row_state = if body.get(F_SEALED_DBS).unwrap_or(0) > 0 {
        RowState::Sealed
    } else {
        RowState::Up
    };
    let none = BTreeMap::new();
    let panes = panes_json(Some(&body), &[], &none, &none, true);
    let host = if whoami.is_empty() { "this host" } else { whoami };
    let note = if row_state == RowState::Sealed {
        "databases held shut awaiting an unseal"
    } else {
        ""
    };
    row_json(host, true, row_state, note, None, body.get(F_UPTIME_S), &panes,
        &services_json(services, now, state.fleet.interval_secs()))
}

/// One watched host: the first entry on it that reads a health body draws the
/// panes, and every other entry on it is a service cell with its own liveness.
fn host_row_json(
    host:       &str,
    group:      &[&(FleetPeer, Vec<ProbeSample>)],
    now:        u64,
    interval:   u64,
)
    -> String
{
    let lead = match group.iter().position(|(p, _)| p.has_token) {
        Some(i) => i,
        None    => 0,
    };
    let (peer, samples) = match group.get(lead) {
        Some(g) => (&g.0, &g.1),
        None    => return row_json(host, false, RowState::Never, "", None, None, "", "[]"),
    };
    let others: Vec<&(FleetPeer, Vec<ProbeSample>)> = group.iter()
        .enumerate()
        .filter(|(i, _)| *i != lead)
        .map(|(_, g)| *g)
        .collect();
    let services = services_json(&others, now, interval);
    let (row_state, note) = peer_state(peer, samples, now, interval);

    if !peer.has_token {
        // Nothing on this host serves a body to this watcher: liveness is all there is.
        let note = if note.is_empty() {
            fmt!("liveness only: no entry for this host carries a health token")
        } else {
            note
        };
        let age = last_ok_age(samples, now);
        return row_json(host, false, row_state, &note, age, None, "", &services);
    }

    let last_read = samples.iter().rev().find(|s| s.body.is_some());
    let age = last_read.map(|s| now.saturating_sub(s.t_secs));
    let (distress, clear) = host_thresholds(group, lead);
    let (panes, uptime) = match (row_state, last_read.and_then(|s| s.body.as_ref())) {
        (RowState::Down, _) | (RowState::Never, _) | (_, None) => (String::new(), None),
        (st, Some(body)) => (
            panes_json(Some(body), samples, &distress, &clear, st.is_fresh()),
            body.get(F_UPTIME_S),
        ),
    };
    row_json(host, false, row_state, &note, age, uptime, &panes, &services)
}

/// The thresholds a host's row is coloured from: the lead entry's, then, for each field the
/// lead does not judge, the first other entry on the host that reads a body and does.
///
/// Two entries on one host read the same body -- jarrah's own figures and its forge copy's
/// stamp ages, say -- and each alarm judges only its own fields, so each cell takes its colour
/// from the entry whose alarm judges that field. A field's clear boundary always comes from the
/// same entry as its distress value, so no dead-band is assembled from two alarms.
fn host_thresholds(
    group:  &[&(FleetPeer, Vec<ProbeSample>)],
    lead:   usize,
)
    -> (BTreeMap<String, i64>, BTreeMap<String, i64>)
{
    let mut distress = BTreeMap::new();
    let mut clear = BTreeMap::new();
    let order = std::iter::once(lead).chain((0..group.len()).filter(|i| *i != lead));
    for i in order {
        let peer = match group.get(i) {
            // An entry with no token never reads a body, so its thresholds judge nothing.
            Some((p, _)) if p.has_token => p,
            _ => continue,
        };
        for (field, d) in &peer.distress {
            if distress.contains_key(field) {
                continue;
            }
            distress.insert(field.clone(), *d);
            if let Some(c) = peer.clear.get(field) {
                clear.insert(field.clone(), *c);
            }
        }
    }
    (distress, clear)
}

fn last_ok_age(samples: &[ProbeSample], now: u64) -> Option<u64> {
    samples.iter().rev().find(|s| s.ok).map(|s| now.saturating_sub(s.t_secs))
}

fn row_json(
    host:       &str,
    local:      bool,
    row_state:  RowState,
    note:       &str,
    age:        Option<u64>,
    uptime:     Option<i64>,
    panes:      &str,
    services:   &str,
)
    -> String
{
    let dim = matches!(row_state, RowState::Stale | RowState::Down | RowState::Never);
    fmt!(
        "{{\"host\":{host},\"local\":{local},\"state\":{st},\"note\":{note},\"dim\":{dim},\
        \"age\":{age},\"uptime\":{uptime},\"panes\":[{panes}],\"services\":{services}}}",
        host     = jstr(host),
        local    = local,
        st       = jstr(row_state.word()),
        note     = jstr(note),
        dim      = dim,
        age      = jopt_u(age),
        uptime   = jopt(uptime),
        panes    = panes,
        services = services,
    )
}

fn services_json(
    group:      &[&(FleetPeer, Vec<ProbeSample>)],
    now:        u64,
    interval:   u64,
)
    -> String
{
    let items: Vec<String> = group.iter().map(|(p, samples)| {
        let (st, note) = peer_state(p, samples, now, interval);
        let probe = samples.last().map(|s| s.probe_ms);
        fmt!(
            "{{\"name\":{name},\"state\":{st},\"note\":{note},\"probe_ms\":{probe},\
            \"age\":{age}}}",
            name  = jstr(&p.name),
            st    = jstr(st.word()),
            note  = jstr(&note),
            probe = jopt_u(probe),
            age   = jopt_u(last_ok_age(samples, now)),
        )
    }).collect();
    fmt!("[{}]", items.join(","))
}

/// The panes of a row. `fresh` false keeps the values and drops every colour,
/// which is how a stale row shows its last numbers without claiming them.
///
/// Every field in the body, and every field a threshold names, is drawn in
/// exactly one place: a known cell, a resident, the row head, or *Other fields*,
/// so nothing the peer said is dropped for want of a label.
fn panes_json(
    body:       Option<&HealthBody>,
    samples:    &[ProbeSample],
    distress:   &BTreeMap<String, i64>,
    clear:      &BTreeMap<String, i64>,
    fresh:      bool,
)
    -> String
{
    let body = match body {
        Some(b) => b,
        None => return String::new(),
    };
    let mut drawn: BTreeSet<String> = HEAD_KEYS.iter().map(|k| k.to_string()).collect();
    let wanted = |c: &CellSpec| c.always || body.get(c.key).is_some() || distress.contains_key(c.key);

    let mut a = Vec::new();
    for c in PANE_A.iter().filter(|c| wanted(*c)) {
        a.push(cell_json(c.key, c.label, c.unit, body, samples, distress, clear, fresh, ""));
        drawn.insert(c.key.to_string());
    }
    for r in body.residents() {
        let capped = r.get(RES_CAP_PCT).is_some();
        let key = resident_key(&r.name, if capped { RES_CAP_PCT } else { RES_RSS_KB });
        let extra = fmt!(
            ",\"procs\":{procs},\"rss\":{rss},\"cap\":{cap},\"pct\":{pct}",
            procs = jopt(r.get(RES_PROCS)),
            rss   = jopt(r.get(RES_RSS_KB)),
            cap   = jopt(r.get(RES_CAP_KB)),
            pct   = jopt(r.get(RES_CAP_PCT)),
        );
        a.push(cell_json(&key, &r.name, "res", body, samples, distress, clear, fresh, &extra));
        for figure in RESIDENT_FIGURES {
            drawn.insert(resident_key(&r.name, figure));
        }
    }

    let mut b = Vec::new();
    for c in PANE_B.iter().filter(|c| wanted(*c)) {
        let extra = if c.key == F_SEALED_DBS {
            drawn.insert(F_SEALED.to_string());
            fmt!(",\"sealed\":{}", jopt(body.get(F_SEALED)))
        } else {
            String::new()
        };
        b.push(cell_json(c.key, c.label, c.unit, body, samples, distress, clear, fresh, &extra));
        drawn.insert(c.key.to_string());
    }

    let mut rest: BTreeSet<&str> = body.fields.keys().map(|k| k.as_str()).collect();
    rest.extend(distress.keys().map(|k| k.as_str()));
    let other: Vec<String> = rest.into_iter()
        .filter(|k| !drawn.contains(*k))
        .map(|k| cell_json(k, k, unit_of(k), body, samples, distress, clear, fresh, ""))
        .collect();

    let mut panes = vec![
        fmt!("{{\"id\":\"a\",\"title\":\"Resources\",\"cells\":[{}]}}", a.join(",")),
        fmt!("{{\"id\":\"b\",\"title\":\"Traffic\",\"cells\":[{}]}}", b.join(",")),
    ];
    if !other.is_empty() {
        panes.push(fmt!(
            "{{\"id\":\"c\",\"title\":\"Other fields\",\"cells\":[{}]}}", other.join(",")));
    }
    panes.join(",")
}

fn cell_json(
    key:        &str,
    label:      &str,
    unit:       &str,
    body:       &HealthBody,
    samples:    &[ProbeSample],
    distress:   &BTreeMap<String, i64>,
    clear:      &BTreeMap<String, i64>,
    fresh:      bool,
    extra:      &str,
)
    -> String
{
    let v = body.get(key);
    let d = distress.get(key).copied();
    let c = clear.get(key).copied();
    let t = match (fresh, v) {
        (false, _) | (_, None) => Tone::Plain,
        // The one binary on the page: a failed self-test means the rest of the
        // traffic pane is decoration, whatever any threshold says.
        (true, Some(v)) if key == F_GUARD_SELFTEST => if v >= 1 { Tone::Green } else { Tone::Red },
        (true, Some(v)) => tone(v, d, c),
    };
    let series: Vec<String> = samples.iter()
        .map(|s| jopt(s.body.as_ref().and_then(|b| b.get(key))))
        .collect();
    fmt!(
        "{{\"k\":{k},\"label\":{label},\"unit\":{unit},\"v\":{v},\"tone\":{tone},\
        \"d\":{d},\"c\":{c},\"s\":[{s}]{extra}}}",
        k     = jstr(key),
        label = jstr(label),
        unit  = jstr(unit),
        v     = jopt(v),
        tone  = jstr(t.word()),
        d     = jopt(d),
        c     = jopt(c),
        s     = series.join(","),
        extra = extra,
    )
}

fn jstr(s: &str) -> String {
    fmt!("\"{}\"", json_escape(s))
}

fn jopt(v: Option<i64>) -> String {
    match v {
        Some(n) => n.to_string(),
        None    => fmt!("null"),
    }
}

fn jopt_u(v: Option<u64>) -> String {
    match v {
        Some(n) => n.to_string(),
        None    => fmt!("null"),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    use crate::srv::{
        admin::{
            host_sampler::HostSampler,
            session::{
                SESSION_COOKIE_NAME,
                encode_session,
            },
            traffic::TrafficRecorder,
        },
        cfg::{
            WatchConfig,
            WatchPeer,
        },
        fleet::{
            Fleet,
            PeerHealth,
        },
    };

    use oxedyne_fe2o3_crypto::keystore::Wallet;
    use oxedyne_fe2o3_net::http::{
        fields::Cookie,
        header::HttpHeadline,
    };

    use std::{
        path::PathBuf,
        sync::{
            Arc,
            RwLock,
        },
    };

    fn mkstate(fleet: Arc<Fleet>) -> Outcome<AdminState> {
        let state = res!(AdminState::new(
            Arc::new(RwLock::new(Wallet::default())),
            PathBuf::from("./wallet.jdat"),
            Some([0u8; 32].to_vec()),
            1,
            None,
            TrafficRecorder::new_shared(0),
            HostSampler::new_shared(),
            res!(crate::srv::admin::guard::new_shared()),
            res!(crate::srv::admin::guard::new_shared()),
            Vec::new(),
            None,
        ));
        Ok(state.with_fleet(fleet))
    }

    fn signed_in(state: &AdminState, scopes: &[&str]) -> Outcome<HeaderFields> {
        let principal = AdminPrincipal {
            name:       fmt!("alice"),
            scopes:     scopes.iter().map(|s| s.to_string()).collect(),
            expires_at: unix_secs() + 3_600,
        };
        let cookie = res!(encode_session(state, &principal));
        let mut h = HeaderFields::default();
        h.insert(
            HeaderName::Cookie,
            HeaderFieldValue::Cookie(vec![Cookie {
                key:    SESSION_COOKIE_NAME.to_string(),
                val:    cookie,
                attrs:  None,
            }]),
            None,
        );
        Ok(h)
    }

    fn status_of(m: &HttpMessage) -> u16 {
        match &m.header.headline {
            HttpHeadline::Response { status } => *status as u16,
            _ => 0,
        }
    }

    fn watched() -> Arc<Fleet> {
        let mut cfg = WatchConfig::default();
        let peer = |name: &str, host: &str, token: Option<&str>| WatchPeer {
            name:       name.to_string(),
            host:       host.to_string(),
            url:        fmt!("https://{}.test/_steel/health", name),
            plain_ok:   false,
            distress:   [(fmt!("mem_pct"), 90)].into_iter().collect(),
            clear:      [(fmt!("mem_pct"), 75)].into_iter().collect(),
            token:      token.map(|t| t.to_string()),
            repeat_secs: None,
        };
        cfg.peers.push(peer("jarrah", "jarrah", Some("the-mesh-token")));
        cfg.peers.push(peer("gateway", "jarrah", None));
        Fleet::new_shared(fmt!("karri"), Some(&cfg))
    }

    /// Signed in is not enough: a principal holding `dashboard.view` alone is refused both the
    /// page and its data, while `dashboard.admin` is served, and a visitor with no session is
    /// sent to sign in.
    #[test]
    fn the_fleet_view_refuses_a_signed_in_non_admin() -> Outcome<()> {
        let state = res!(mkstate(watched()));

        let viewer = res!(signed_in(&state, &["dashboard.view"]));
        assert_eq!(status_of(&render_fleet_page(&state, &viewer)), 403);
        assert_eq!(status_of(&render_fleet_json(&state, &viewer)), 403);

        let admin = res!(signed_in(&state, &["dashboard.admin"]));
        assert_eq!(status_of(&render_fleet_page(&state, &admin)), 200);
        assert_eq!(status_of(&render_fleet_json(&state, &admin)), 200);

        let wildcard = res!(signed_in(&state, &["*"]));
        assert_eq!(status_of(&render_fleet_json(&state, &wildcard)), 200);

        let nobody = HeaderFields::default();
        assert_eq!(status_of(&render_fleet_page(&state, &nobody)), 303, "sent to sign in");
        assert_eq!(status_of(&render_fleet_json(&state, &nobody)), 401);
        Ok(())
    }

    /// The document groups a host's entries into one row, colours from that peer's
    /// thresholds, and never carries a token.
    #[test]
    fn the_document_groups_by_host_and_colours_from_the_peer_thresholds() -> Outcome<()> {
        let fleet = watched();
        let now = unix_secs();
        let mut body = HealthBody::new();
        body.set(F_MEM_PCT, 80);
        body.set(F_GUARD_SELFTEST, 1);
        res!(fleet.record(0, ProbeSample {
            t_secs: now, ok: true, probe_ms: 120, body: Some(body), health: PeerHealth::Up }));
        res!(fleet.record(1, ProbeSample {
            t_secs: now, ok: true, probe_ms: 45, body: None, health: PeerHealth::Up }));
        let state = res!(mkstate(fleet));
        let json = fleet_json(&state);

        assert!(json.contains("\"host\":\"karri\",\"local\":true"), "own row first: {}", json);
        assert_eq!(json.matches("\"host\":\"jarrah\"").count(), 1,
            "the gateway must share jarrah's row, not make its own: {}", json);
        assert!(json.contains("\"name\":\"gateway\",\"state\":\"up\""), "{}", json);
        assert!(json.contains("\"k\":\"mem_pct\",\"label\":\"Memory\",\"unit\":\"pct\",\"v\":80,\
            \"tone\":\"amber\",\"d\":90,\"c\":75"), "80 sits between clear 75 and distress 90: {}",
            json);
        assert!(!json.contains("the-mesh-token"), "a token must never reach the page");
        Ok(())
    }

    /// A host watched by two entries that read one body -- jarrah's figures and its forge copy's
    /// stamp ages -- draws both on its one row, each field coloured from the entry that judges
    /// it, the lead's own thresholds standing where both name a field.
    #[test]
    fn a_hosts_row_colours_each_field_from_the_entry_that_judges_it() -> Outcome<()> {
        let mut cfg = WatchConfig::default();
        let entry = |name: &str, distress: &[(&str, i64)], clear: &[(&str, i64)]| WatchPeer {
            name:       name.to_string(),
            host:       fmt!("jarrah"),
            url:        fmt!("https://oxedyne.test/_steel/health"),
            plain_ok:   false,
            distress:   distress.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            clear:      clear.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            token:      Some(fmt!("the-mesh-token")),
            repeat_secs: None,
        };
        cfg.peers.push(entry("jarrah", &[("mem_pct", 90)], &[("mem_pct", 75)]));
        cfg.peers.push(entry("jarrah forge copy",
            &[("forge_state_age_s", 10_800), ("forge_repos_age_s", 10_800), ("mem_pct", 50)],
            &[("forge_state_age_s", 7_200), ("forge_repos_age_s", 7_200)]));
        let fleet = Fleet::new_shared(fmt!("karri"), Some(&cfg));
        let now = unix_secs();
        let mut body = HealthBody::new();
        body.set(F_MEM_PCT, 80);
        body.set("forge_state_age_s", 12_000);
        body.set("forge_repos_age_s", 900);
        for i in 0..2 {
            res!(fleet.record(i, ProbeSample { t_secs: now, ok: true, probe_ms: 90,
                body: Some(body.clone()), health: PeerHealth::Up }));
        }
        let json = fleet_json(&res!(mkstate(fleet)));

        assert_eq!(json.matches("\"host\":\"jarrah\"").count(), 1, "{}", json);
        assert!(json.contains("{\"k\":\"forge_state_age_s\",\"label\":\"forge_state_age_s\",\
            \"unit\":\"secs\",\"v\":12000,\"tone\":\"red\",\"d\":10800,\"c\":7200"),
            "a stale stamp is red by the forge copy's own thresholds: {}", json);
        assert!(json.contains("{\"k\":\"forge_repos_age_s\",\"label\":\"forge_repos_age_s\",\
            \"unit\":\"secs\",\"v\":900,\"tone\":\"green\""), "{}", json);
        assert!(json.contains("\"k\":\"mem_pct\",\"label\":\"Memory\",\"unit\":\"pct\",\"v\":80,\
            \"tone\":\"amber\",\"d\":90,\"c\":75"),
            "where both entries name a field, the lead's thresholds stand: {}", json);
        assert!(json.contains("\"name\":\"jarrah forge copy\",\"state\":\"up\""), "{}", json);
        assert!(json.contains("\"link_down\":false"), "{}", json);
        Ok(())
    }

    /// A field no pane knows is shown under Other, formatted from its suffix and coloured from
    /// its threshold, and a field a threshold names that the body lacks is shown empty, so a
    /// threshold with nothing to judge is visible rather than silent.
    #[test]
    fn a_field_the_page_does_not_know_is_shown_not_dropped() -> Outcome<()> {
        let mut body = HealthBody::new();
        body.set(F_MEM_PCT, 40);
        body.set(F_UPTIME_S, 3_600);
        body.set(F_SEALED, 1);
        body.set("forge_state_age_s", 7_300);
        let mut distress = BTreeMap::new();
        distress.insert(fmt!("forge_state_age_s"), 7_200);
        distress.insert(fmt!("forge_repos_age_s"), 86_400);
        let panes = panes_json(Some(&body), &[], &distress, &BTreeMap::new(), true);

        assert!(panes.contains("\"id\":\"c\",\"title\":\"Other fields\""), "{}", panes);
        assert!(panes.contains("{\"k\":\"forge_state_age_s\",\"label\":\"forge_state_age_s\",\
            \"unit\":\"secs\",\"v\":7300,\"tone\":\"red\",\"d\":7200"), "{}", panes);
        assert!(panes.contains("{\"k\":\"forge_repos_age_s\",\"label\":\"forge_repos_age_s\",\
            \"unit\":\"secs\",\"v\":null"), "{}", panes);
        // With no `sealed_dbs` to carry it, the raw seal falls to Other rather than vanishing.
        assert!(panes.contains("{\"k\":\"sealed\",\"label\":\"sealed\""), "{}", panes);
        assert!(!panes.contains("\"k\":\"uptime_s\""), "uptime is the row head's, not a cell's");

        // Once `sealed_dbs` is there, its cell carries `sealed` and Other does not repeat it.
        body.set(F_SEALED_DBS, 0);
        let panes = panes_json(Some(&body), &[], &distress, &BTreeMap::new(), true);
        assert!(panes.contains("\"k\":\"sealed_dbs\",\"label\":\"Seal\""), "{}", panes);
        assert!(panes.contains("\"sealed\":1"), "{}", panes);
        assert!(!panes.contains("{\"k\":\"sealed\","), "{}", panes);
        Ok(())
    }
}
