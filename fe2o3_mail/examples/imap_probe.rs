//! Probe a live IMAP server: connect over TLS, read the greeting, list the
//! capabilities. With no credentials it stops there, checking only that a
//! junk login is refused cleanly. With them it selects INBOX and prints the
//! newest few headers.
//!
//! ```text
//! cargo run -p oxedyne_fe2o3_mail --example imap_probe -- imap.gmail.com 993 [user] [app-password]
//! ```

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::imap::client::{
    FetchWhat,
    ImapClient,
    ImapConfig,
    Security,
};

#[tokio::main]
async fn main() -> Outcome<()> {
    let args: Vec<String> = std::env::args().collect();
    let host = args.get(1).cloned().unwrap_or_else(|| fmt!("imap.gmail.com"));
    let port: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(993);

    let cfg = ImapConfig::new(host.clone(), port, Security::ImplicitTls);
    println!("Connecting to {}:{} over TLS...", host, port);
    let mut c = res!(ImapClient::connect(&cfg).await);
    println!("Connected. Capabilities: {:?}", res!(c.capability().await));

    let user = match args.get(3) {
        Some(u) => u.clone(),
        None => {
            println!("No credentials given; checking that a junk login is refused.");
            match c.login("nobody@example.invalid", "not-a-password").await {
                Ok(())  => println!("UNEXPECTED: the server accepted a junk login."),
                Err(e)  => println!("Refused, as it should be: {}", e),
            }
            return Ok(());
        }
    };
    let pass = args.get(4).cloned().unwrap_or_default();

    res!(c.login(&user, &pass).await);
    println!("Logged in as {}.", user);
    for mb in res!(c.list("", "*").await) {
        println!("  mailbox: {} (delim {:?}, {:?})", mb.name, mb.delimiter, mb.attrs);
    }
    let st = res!(c.select("INBOX").await);
    println!("INBOX: {} messages, uidvalidity {}, uidnext {}",
        st.exists, st.uid_validity, st.uid_next);

    let from = st.uid_next.saturating_sub(3).max(1);
    let uids = res!(c.uid_search(&fmt!("UID {}:*", from)).await);
    for msg in res!(c.uid_fetch(&uids, FetchWhat::Headers).await) {
        let head = String::from_utf8_lossy(&msg.body);
        let subj = head.lines()
            .find(|l| l.to_lowercase().starts_with("subject:"))
            .unwrap_or("(no subject)")
            .to_string();
        println!("  uid {} [{}] {} ({} bytes)",
            msg.uid, msg.flags.join(","), subj.trim(), msg.size);
    }
    res!(c.logout().await);
    Ok(())
}
