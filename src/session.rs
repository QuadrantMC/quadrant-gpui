//! The Quadrant ID account every page shares: whether someone is signed in,
//! and the host's background workers that only make sense while they are.

use gpui::{App, AppContext as _, Context, Entity, Global};
use quadrant_core::account::id::AccountInfo;
use quadrant_host::ErrorCode;

use crate::{
    backend::Backend,
    events,
    i18n::describe,
    loadable::{Loadable, Reload},
};

/// `Ready(None)` is signed out; `Failed` is an account that couldn't be read,
/// e.g. while offline.
pub type Account = Loadable<Option<AccountInfo>>;

pub struct Session {
    account: Account,
    /// Whether the host's notification, settings sync and token refresh
    /// workers run. The desktop app starts them at launch whenever it is
    /// built with Quadrant ID; here they follow the sign-in.
    workers: bool,
    /// The newest account check; replacing it drops a stale one's answer.
    checking: Reload,
}

struct GlobalSession(Entity<Session>);

impl Global for GlobalSession {}

impl Session {
    /// The app's session, created and first checked on first use.
    pub fn global(cx: &mut App) -> Entity<Session> {
        if let Some(session) = cx.try_global::<GlobalSession>() {
            return session.0.clone();
        }
        let session = cx.new(Session::new);
        cx.set_global(GlobalSession(session.clone()));
        session
    }

    fn new(cx: &mut Context<Self>) -> Self {
        // Raised after a sign-in and whenever a token refresh changes things.
        events::listen(&["recheckAccountToken"], cx, |this, _, cx| this.refresh(cx));
        let mut session = Self {
            account: Loadable::Idle,
            workers: false,
            checking: Reload::default(),
        };
        session.refresh(cx);
        session
    }

    pub fn account(&self) -> &Account {
        &self.account
    }

    /// The signed-in account, if there is one.
    pub fn signed_in(&self) -> Option<&AccountInfo> {
        match &self.account {
            Loadable::Ready(Some(info)) => Some(info),
            _ => None,
        }
    }

    /// Asks the host who is signed in, showing the check while it runs.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.account = Loadable::Loading;
        self.check(cx);
    }

    /// Asks again while the account shown stays on screen.
    pub fn revalidate(&mut self, cx: &mut Context<Self>) {
        if self.checking.ask() {
            self.account.reload();
            self.check(cx);
        }
    }

    fn check(&mut self, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, |host| async move {
            match host.get_account_info().await {
                Ok(info) => Ok(Some(info)),
                Err(error) if is_signed_out(&error) => Ok(None),
                Err(error) => Err(error),
            }
        });
        self.checking.start(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.account = Loadable::from_result(result);
                this.match_workers(cx);
                if this.checking.landed() {
                    this.check(cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Starts the workers once signed in and stops them once signed out. An
    /// account that couldn't be read leaves them as they are.
    fn match_workers(&mut self, cx: &mut Context<Self>) {
        let wanted = match &self.account {
            Loadable::Ready(account) => account.is_some(),
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => return,
        };
        if wanted == self.workers {
            return;
        }
        self.workers = wanted;
        let task = Backend::global(cx).run(cx, move |host| async move {
            if wanted {
                host.start_background_workers().await
            } else {
                host.stop_background_workers().await
            }
        });
        cx.spawn(async move |_, _| {
            if let Err(error) = task.await {
                log::warn!("Quadrant ID workers: {}", describe(error));
            }
        })
        .detach();
    }

    /// Shows `info` as signed in without asking the host or starting the
    /// workers, for screenshots of the signed-in pages.
    #[cfg(feature = "screenshot")]
    pub fn preview_signed_in(&mut self, info: AccountInfo, cx: &mut Context<Self>) {
        self.checking = Reload::default();
        self.account = Loadable::Ready(Some(info));
        cx.notify();
    }
}

fn is_signed_out(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<ErrorCode>() == Some(&ErrorCode::SignedOut))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadrant_host::{QuadrantHost, QuadrantHostOptions};

    #[test]
    fn a_keyring_without_a_token_reads_as_signed_out() {
        let temp = tempfile::tempdir().unwrap();
        let mut options = QuadrantHostOptions::new(temp.path().to_path_buf(), "", "", "");
        options.keyring_service_name = "quadrant-gpui-tests-signed-out".into();
        let host = QuadrantHost::new(options).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime.block_on(host.get_account_info()).unwrap_err();
        assert!(is_signed_out(&error), "{error:#}");
        assert!(!is_signed_out(&anyhow::anyhow!("offline")));
    }
}
