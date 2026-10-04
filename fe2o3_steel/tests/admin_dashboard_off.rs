//! A vhost that has switched the dashboard off answers 404 for `/admin` and everything under it.
//!
//! The dashboard is one surface for the whole process, and until a vhost could say otherwise it was
//! served on every name Steel answers to: ontheism.org, a static site with no database and nothing
//! to administer, showed the operator's login page. Two vhosts go through `handle_https` here, both
//! backed by a real dashboard state and a web root holding a file at `/admin/page.html`. The one
//! left at its default serves the dashboard; the one with `admin_dashboard` off serves neither the
//! dashboard nor the file, for a `GET` or a `POST`, at `/admin`, `/admin/`, and any depth below.
//! Its neighbours (`/admin.html`, `/administrator`) are not the prefix and are still its own.
//!
//! The setting is read from config as a boolean that defaults to on, so every config written
//! before it existed keeps its dashboard.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

mod vhost_rig;

use vhost_rig::{
    Rig,
    Scratch,
    Site,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_steel::srv::cfg::VhostConfig;

const ON:  &str = "dash.test";
const OFF: &str = "public.test";

#[test]
fn a_vhost_with_the_dashboard_off_has_no_admin() -> Outcome<()> {
    let web = res!(Scratch::new("admin_off"));
    res!(web.put("index.html", "<p>home</p>"));
    res!(web.put("admin.html", "<p>a page that is merely named admin</p>"));
    res!(web.put("admin/page.html", "<p>a file under the prefix</p>"));
    res!(web.put("administrator", "neighbour"));
    let on = Site::new(ON, &web.0);
    let mut off = Site::new(OFF, &web.0);
    off.admin_dashboard = false;
    let rig = res!(Rig::new(vec![on, off]));
    let runtime = res!(vhost_rig::runtime());
    runtime.block_on(async {
        // The vhost left at its default still serves the dashboard, so a 404 on the other is the
        // setting and not a rig that never had one.
        for path in ["/admin", "/admin/", "/admin/login", "/admin/database"] {
            let r = res!(rig.fetch(ON, "GET", path, &[], "").await);
            assert_ne!(r.status(), 404, "the default vhost must serve the dashboard at {}: {:?}",
                path, r);
        }
        // The vhost with it off answers 404 at every depth, and the file under the prefix, which
        // a plain static server would hand over, stays unread.
        for path in ["/admin", "/admin/", "/admin/login", "/admin/database", "/admin/database/x",
            "/admin/page.html", "/admin/a/b/c", "/admin?next=/x", "/admin/page.html?x=1"]
        {
            let r = res!(rig.fetch(OFF, "GET", path, &[], "").await);
            assert_eq!(r.status(), 404, "GET {} must be a 404: {:?}", path, r);
            assert!(!r.body.contains("a file under the prefix"), "GET {} leaked the file: {:?}",
                path, r);
            assert!(!r.body.to_lowercase().contains("passphrase"),
                "GET {} showed a login page: {:?}", path, r);
        }
        for path in ["/admin", "/admin/login", "/admin/logout"] {
            let r = res!(rig.fetch(OFF, "POST", path,
                &["Content-Type: application/x-www-form-urlencoded"], "passphrase=x").await);
            assert_eq!(r.status(), 404, "POST {} must be a 404: {:?}", path, r);
        }
        let r = res!(rig.fetch(OFF, "HEAD", "/admin", &[], "").await);
        assert_eq!(r.status(), 404, "HEAD /admin must be a 404: {:?}", r);
        // What is not under the prefix is untouched.
        for (path, want) in [("/", "home"), ("/admin.html", "merely named admin"),
            ("/administrator", "neighbour")]
        {
            let r = res!(rig.fetch(OFF, "GET", path, &[], "").await);
            assert_eq!(r.status(), 200, "GET {} must still be served: {:?}", path, r);
            assert!(r.body.contains(want), "GET {}: {:?}", path, r);
        }
        Ok(())
    })
}

fn vhost(extra: &str) -> Outcome<VhostConfig> {
    let text = fmt!("{{\"hostnames\": [\"ontheism.org\"] {}}}", extra);
    match res!(Dat::decode_string(text)) {
        Dat::Map(m) => VhostConfig::from_datmap(&m),
        _ => Err(err!("The test config is not a map."; Test, Invalid)),
    }
}

#[test]
fn admin_dashboard_defaults_to_on_and_must_be_a_boolean() -> Outcome<()> {
    assert!(res!(vhost("")).admin_dashboard,
        "a vhost written before the setting existed must keep its dashboard");
    assert!(res!(vhost(", \"admin_dashboard\": (true)")).admin_dashboard);
    assert!(!res!(vhost(", \"admin_dashboard\": (false)")).admin_dashboard);
    assert!(vhost(", \"admin_dashboard\": \"no\"").is_err(),
        "a non-boolean 'admin_dashboard' must be refused, not read as on");
    Ok(())
}
