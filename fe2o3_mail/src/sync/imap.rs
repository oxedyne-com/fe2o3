//! [`MailSession`] over the native, tokio-driven [`ImapClient`], so the gateway needs no adapter
//! of its own.

use crate::sync::{
    AppendUid,
    CopyUid,
    FetchChanged,
    FetchWhat,
    FetchedMessage,
    MailSession,
    MailboxInfo,
    MailboxStatus,
    SelectWith,
    UidSet,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::imap::client::ImapClient;


impl MailSession for ImapClient {
    async fn capabilities(&mut self) -> Outcome<Vec<String>> {
        Ok(res!(self.capability().await).to_vec())
    }

    async fn list(&mut self) -> Outcome<Vec<MailboxInfo>> {
        ImapClient::list(self, "", "*").await
    }

    async fn create(&mut self, mailbox: &str) -> Outcome<()> {
        ImapClient::create(self, mailbox).await
    }

    async fn select(&mut self, mailbox: &str, with: &SelectWith) -> Outcome<MailboxStatus> {
        self.select_with(mailbox, with).await
    }

    async fn uid_fetch(&mut self, uids: &UidSet, what: FetchWhat) -> Outcome<Vec<FetchedMessage>> {
        ImapClient::uid_fetch(self, uids, what).await
    }

    async fn uid_fetch_changed(&mut self, uids: &UidSet, what: FetchWhat, modseq: u64)
        -> Outcome<FetchChanged>
    {
        ImapClient::uid_fetch_changed(self, uids, what, modseq).await
    }

    async fn uid_search(&mut self, criteria: &str) -> Outcome<Vec<u32>> {
        ImapClient::uid_search(self, criteria).await
    }

    async fn uid_store_flags(&mut self, uids: &UidSet, add: &[&str], remove: &[&str]) -> Outcome<()> {
        ImapClient::uid_store_flags(self, uids, add, remove).await
    }

    async fn uid_copy(&mut self, uids: &UidSet, to: &str) -> Outcome<Option<CopyUid>> {
        ImapClient::uid_copy(self, uids, to).await
    }

    async fn uid_move(&mut self, uids: &UidSet, to: &str) -> Outcome<Option<CopyUid>> {
        ImapClient::uid_move(self, uids, to).await
    }

    async fn uid_expunge(&mut self, uids: &UidSet) -> Outcome<()> {
        ImapClient::uid_expunge(self, uids).await
    }

    async fn append(&mut self, mailbox: &str, flags: &[&str], raw: &[u8]) -> Outcome<Option<AppendUid>> {
        ImapClient::append(self, mailbox, flags, raw).await
    }
}
