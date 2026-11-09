//! Bounded, data-only requests. AppKit objects never leave the main thread.
use crate::execution::Live;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

const LIMIT: usize = 16;
type Wake = Arc<dyn Fn() + Send + Sync>;

#[cfg(feature = "test-tools")]
struct OwnedPending(Arc<std::sync::atomic::AtomicUsize>);
#[cfg(feature = "test-tools")]
impl OwnedPending {
    fn reserve(count: &Arc<std::sync::atomic::AtomicUsize>) -> Result<Self,String> {
        count.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|n.checked_add(1))
            .map_err(|_|"Main-thread service observation capacity exhausted.")?;
        Ok(Self(count.clone()))
    }
}
#[cfg(feature = "test-tools")]
impl Drop for OwnedPending {
    fn drop(&mut self) {
        let _=self.0.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|n.checked_sub(1));
    }
}

/// The state owner retains this guard until delivery/retirement. Queued
/// requests lose authority when that owner drops the execution context.
#[derive(Default)]
pub struct Permit(Arc<AtomicBool>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
impl Permit {
    pub fn live(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}
#[derive(Clone)]
enum NativeDispatch {
    Workspace(Arc<crate::workspace::Dispatch>),
    Snippet(Arc<crate::paste::Dispatch>),
    UtilityHide(Arc<crate::paste::Dispatch>),
}
impl NativeDispatch {
    fn preparing(&self) -> bool {
        match self {
            Self::Workspace(d) => d.is_preparing(),
            Self::Snippet(d) => d.preparing(),
            Self::UtilityHide(d) => d.live(),
        }
    }
}
enum Operation {
    Clipboard,
    CreationPrefill,
    Hide(crate::connected::HideToken),
}
enum Reply {
    Clipboard(Option<String>),
    Hidden(crate::connected::HideKey),
}
struct Request {
    #[cfg(feature = "test-tools")]
    _owned_pending: OwnedPending,
    operation: Operation,
    deadline: Option<Instant>,
    live: Live,
    revoked: Arc<AtomicBool>,
    // None is exclusively a creation request; it owns a real draft permit,
    // never an invented workspace execution.
    dispatch: Option<NativeDispatch>,
    complete: Option<Wake>,
    reply: mpsc::SyncSender<Result<Reply, String>>,
}
impl Request {
    fn live(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
            && self.dispatch.as_ref().is_none_or(NativeDispatch::preparing)
            && !crate::deadline::expired(self.deadline)
            && (self.live)()
    }
}
#[derive(Clone)]
pub struct Client {
    #[cfg(feature = "test-tools")]
    owned_pending: Arc<std::sync::atomic::AtomicUsize>,
    pub clipboard_capture: crate::clipboard_history::CapturePort,
    pub system: crate::system_search::main_bridge::Client,
    keyboard: Arc<Mutex<Option<crate::keyboard_service::Client>>>,
    sender: mpsc::SyncSender<Request>,
    wake: Arc<Mutex<Option<Wake>>>,
    paste: crate::paste::Client,
}
pub struct Server {
    #[cfg(feature = "test-tools")]
    owned_pending: Arc<std::sync::atomic::AtomicUsize>,
    pub system: crate::system_search::main_bridge::Main,
    receiver: mpsc::Receiver<Request>,
    wake: Arc<Mutex<Option<Wake>>>,
    paste: crate::paste::Server,
}
pub fn channel() -> (Client, Server) {
    #[cfg(feature = "test-tools")]
    let owned_pending=Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (system_client, system_main) = crate::system_search::main_bridge::channel();
    let (sender, receiver) = mpsc::sync_channel(LIMIT);
    let wake = Arc::new(Mutex::new(None));
    let (paste_client, paste_server) = crate::paste::channel(wake.clone());
    (
        Client {
            #[cfg(feature = "test-tools")]
            owned_pending: owned_pending.clone(),
            clipboard_capture: Default::default(),
            system: system_client,
            keyboard: Arc::new(Mutex::new(None)),
            sender,
            wake: wake.clone(),
            paste: paste_client,
        },
        Server {
            #[cfg(feature = "test-tools")]
            owned_pending,
            system: system_main,
            receiver,
            wake,
            paste: paste_server,
        },
    )
}
/// Nonblocking owner-side receipt; no thread is spawned to wait for creation.
pub struct Prefill(mpsc::Receiver<Result<Reply, String>>);
impl Prefill {
    pub fn take(&self) -> Option<Result<Option<String>, String>> {
        match self.0.try_recv() {
            Ok(Ok(Reply::Clipboard(text))) => Some(Ok(text)),
            Ok(Ok(_)) => Some(Err("Invalid creation service reply.".into())),
            Ok(Err(error)) => Some(Err(error)),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("Creation service stopped.".into())),
        }
    }
}
impl Client {
    /// A Utility may already have persisted data or completed a copy before
    /// hiding. It still owns the same bounded, cancellable execution ticket.
    pub fn hide_utility(&self, token:crate::connected::HideToken, deadline:Option<Instant>, live:Live, revoked:Arc<AtomicBool>, dispatch:Arc<crate::paste::Dispatch>)->Result<(),String>{
        let expected=token.key.clone();
        match self.request(Operation::Hide(token),deadline,live,revoked,NativeDispatch::UtilityHide(dispatch))?{Reply::Hidden(key) if key==expected=>Ok(()),_=>Err("Invalid Utility window hide acknowledgement.".into())}
    }
    pub fn keyboard(&self) -> Option<crate::keyboard_service::Client> {
        self.keyboard
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn attach_keyboard(&self, client: crate::keyboard_service::Client) {
        *self.keyboard.lock().unwrap_or_else(|e| e.into_inner()) = Some(client);
    }
    pub fn paste(&self) -> crate::paste::Client {
        self.paste.clone()
    }
    pub fn begin_prefill(
        &self,
        deadline: Instant,
        live: Live,
        revoked: Arc<AtomicBool>,
        complete: Wake,
    ) -> Result<Prefill, String> {
        let (reply, receive) = mpsc::sync_channel(1);
        let request = Request {
            #[cfg(feature = "test-tools")]
            _owned_pending: OwnedPending::reserve(&self.owned_pending)?,
            operation: Operation::CreationPrefill,
            deadline: Some(deadline),
            live,
            revoked,
            dispatch: None,
            complete: Some(complete),
            reply,
        };
        if !request.live() {
            return Err("Creation request expired or was cancelled.".into());
        }
        self.sender
            .try_send(request)
            .map_err(|_| "Main-thread service is busy or stopped.".to_string())?;
        if let Some(wake) = self.wake.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            wake();
        }
        Ok(Prefill(receive))
    }
    pub fn attach_wake(&self, wake: Wake) {
        self.system.attach_wake(wake.clone());
        *self.wake.lock().unwrap_or_else(|p| p.into_inner()) = Some(wake.clone());
        wake();
    }
    /// Only called on a worker, after a fresh execution-owner grant. Waiting
    /// never holds the owner/main loop, and each request has its own reply.
    pub fn clipboard(
        &self,
        deadline: impl Into<Option<Instant>>,
        live: Live,
        revoked: Arc<AtomicBool>,
        dispatch: Arc<crate::workspace::Dispatch>,
    ) -> Result<Option<String>, String> {
        match self.request(
            Operation::Clipboard,
            deadline.into(),
            live,
            revoked,
            NativeDispatch::Workspace(dispatch),
        )? {
            Reply::Clipboard(value) => Ok(value),
            _ => Err("Invalid clipboard service reply.".into()),
        }
    }
    pub fn clipboard_for_snippet(
        &self,
        deadline: impl Into<Option<Instant>>,
        live: Live,
        revoked: Arc<AtomicBool>,
        dispatch: Arc<crate::paste::Dispatch>,
    ) -> Result<Option<String>, String> {
        match self.request(
            Operation::Clipboard,
            deadline.into(),
            live,
            revoked,
            NativeDispatch::Snippet(dispatch),
        )? {
            Reply::Clipboard(value) => Ok(value),
            _ => Err("Invalid clipboard service reply.".into()),
        }
    }
    pub fn hide(
        &self,
        token: crate::connected::HideToken,
        deadline: impl Into<Option<Instant>>,
        live: Live,
        revoked: Arc<AtomicBool>,
        dispatch: Arc<crate::workspace::Dispatch>,
    ) -> Result<(), String> {
        let expected = token.key.clone();
        match self.request(
            Operation::Hide(token),
            deadline.into(),
            live,
            revoked,
            NativeDispatch::Workspace(dispatch),
        )? {
            Reply::Hidden(key) if key == expected => Ok(()),
            _ => Err("Invalid window hide acknowledgement.".into()),
        }
    }
    fn request(
        &self,
        operation: Operation,
        deadline: Option<Instant>,
        live: Live,
        revoked: Arc<AtomicBool>,
        dispatch: NativeDispatch,
    ) -> Result<Reply, String> {
        let (reply, receive) = mpsc::sync_channel(1);
        let dispatch = Arc::new(dispatch);
        let request = Request {
            #[cfg(feature = "test-tools")]
            _owned_pending: OwnedPending::reserve(&self.owned_pending)?,
            operation,
            deadline,
            live: live.clone(),
            revoked: revoked.clone(),
            dispatch: Some(dispatch.as_ref().clone()),
            complete: None,
            reply,
        };
        if !request.live() {
            return Err("Request expired or was cancelled.".into());
        }
        self.sender
            .try_send(request)
            .map_err(|_| "Main-thread service is busy or stopped.".to_string())?;
        if let Some(wake) = self.wake.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            wake();
        }
        loop {
            if !dispatch.preparing()
                || revoked.load(Ordering::Acquire)
                || !(live)()
                || crate::deadline::expired(deadline)
            {
                return Err("Request expired or was cancelled.".into());
            }
            match receive.recv_timeout(crate::deadline::poll(deadline)) {
                Ok(value) => {
                    if !dispatch.preparing()
                        || revoked.load(Ordering::Acquire)
                        || !(live)()
                        || crate::deadline::expired(deadline)
                    {
                        return Err("Request expired or was cancelled.".into());
                    }
                    return value;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("Main-thread service stopped.".into()),
            }
        }
    }
}
impl Server {
    /// Queued plus currently executing requests, including requests whose
    /// caller has cancelled but which the real owner has not yet retired.
    #[cfg(feature = "test-tools")]
    pub fn owned_pending(&self) -> usize {
        self.owned_pending.load(Ordering::Acquire)
    }
    pub fn take_pastes(&self) -> Vec<crate::paste::Ticket> {
        self.paste.take()
    }
    pub fn attach_wake(&self, wake: Wake) {
        self.system.attach_wake(wake.clone());
        *self.wake.lock().unwrap_or_else(|p| p.into_inner()) = Some(wake.clone());
        wake(); // Also handles requests submitted before attachment.
    }
    /// Main-only adapter supplies the actual pasteboard reader. Snapshot a
    /// bounded batch so flooding cannot starve native event processing.
    pub fn drain(&self, read: impl FnMut() -> Result<Option<String>, String>) {
        self.drain_with_hide(read, |_| Err("No native window is available.".into()));
    }
    pub fn drain_with_hide(
        &self,
        read: impl FnMut() -> Result<Option<String>, String>,
        hide: impl FnMut(&crate::connected::HideToken) -> Result<crate::connected::HideKey, String>,
    ) {
        self.drain_with_prefill(read, hide, || Err("Creation service unavailable.".into()));
    }
    pub fn drain_with_prefill(
        &self,
        mut read: impl FnMut() -> Result<Option<String>, String>,
        mut hide: impl FnMut(&crate::connected::HideToken) -> Result<crate::connected::HideKey, String>,
        mut prefill: impl FnMut() -> Result<Option<String>, String>,
    ) {
        self.system.drain();
        for _ in 0..LIMIT {
            let Ok(request) = self.receiver.try_recv() else {
                break;
            };
            if !request.live() {
                continue;
            }
            let value = match &request.operation {
                Operation::Clipboard => read().map(Reply::Clipboard),
                Operation::CreationPrefill => prefill().map(Reply::Clipboard),
                Operation::Hide(token) => hide(token).map(Reply::Hidden),
            };
            if request.live() {
                let _ = request.reply.try_send(value);
                if let Some(wake) = request.complete {
                    wake();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    fn discard_attachment_wakes(rx: &mpsc::Receiver<()>) {
        // Attachment notifications are synchronous and precede every worker.
        while rx.try_recv().is_ok() {}
    }
    #[test]
    fn initial_attachment_wakes_cannot_stand_in_for_worker_enqueue() {
        let (client, server) = channel();
        let (tx, rx) = mpsc::channel();
        server.attach_wake(Arc::new(move || { let _ = tx.send(()); }));
        discard_attachment_wakes(&rx);
        let permit = Permit::default();
        let revoked = permit.live();
        let (entered, held) = mpsc::sync_channel(0);
        let (release, resume) = mpsc::sync_channel(0);
        let deadline = Instant::now() + Duration::from_secs(5);
        let worker = thread::spawn(move || {
            entered.send(()).unwrap();
            resume.recv().unwrap();
            client.clipboard(deadline, Arc::new(|| true), revoked, Arc::default())
        });
        held.recv().unwrap();
        let attachment_looked_like_work = rx.try_recv().is_ok();
        let mut reads_before_submit = 0;
        server.drain(|| { reads_before_submit += 1; Ok(None) });
        // Always retire the actual worker before asserting the old premise.
        release.send(()).unwrap();
        rx.recv().unwrap();
        let mut reads_after_submit = 0;
        server.drain(|| { reads_after_submit += 1; Ok(Some("owned queued clipboard".into())) });
        assert_eq!(worker.join().unwrap().unwrap(), Some("owned queued clipboard".into()));
        eprintln!("attachment_looked_like_work={attachment_looked_like_work}; reads_before_submit={reads_before_submit}; reads_after_submit={reads_after_submit}");
        assert_eq!(reads_before_submit, 0);
        assert_eq!(reads_after_submit, 1);
        assert!(!attachment_looked_like_work, "An attachment wake was mistaken for queued clipboard work");
    }
    #[cfg(feature = "test-tools")]
    #[test]
    fn owned_pending_tracks_queue_failure_cancel_reply_and_owner_drop() {
        let (client,server)=channel();
        let mut retained=Vec::new();
        let mut permits=Vec::new();
        for _ in 0..LIMIT {
            let permit=Permit::default();
            retained.push(client.begin_prefill(Instant::now()+Duration::from_secs(5),Arc::new(||true),permit.live(),Arc::new(||{})).unwrap());
            permits.push(permit);
        }
        assert_eq!(server.owned_pending(),LIMIT);
        let overflow=Permit::default();
        assert!(client.begin_prefill(Instant::now()+Duration::from_secs(5),Arc::new(||true),overflow.live(),Arc::new(||{})).is_err());
        assert_eq!(server.owned_pending(),LIMIT);
        drop(permits); drop(retained);
        // Cancelled callers do not erase queued owner work prematurely.
        assert_eq!(server.owned_pending(),LIMIT);
        server.drain(||panic!("Cancelled requests must not read."));
        assert_eq!(server.owned_pending(),0);
        let permit=Permit::default();
        let reply=client.begin_prefill(Instant::now()+Duration::from_secs(5),Arc::new(||true),permit.live(),Arc::new(||{})).unwrap();
        server.drain_with_prefill(||Ok(None),|_|unreachable!(),||{
            assert_eq!(server.owned_pending(),1);
            Ok(Some("owned reply".into()))
        });
        assert_eq!(reply.take().unwrap().unwrap(),Some("owned reply".into()));
        assert_eq!(server.owned_pending(),0);
        let _queued=client.begin_prefill(Instant::now()+Duration::from_secs(5),Arc::new(||true),permit.live(),Arc::new(||{})).unwrap();
        let counter=client.owned_pending.clone();
        assert_eq!(counter.load(Ordering::Acquire),1);
        drop(server);
        assert_eq!(counter.load(Ordering::Acquire),0);
        counter.store(usize::MAX,Ordering::Release);
        assert!(OwnedPending::reserve(&counter).is_err());
        assert_eq!(counter.load(Ordering::Acquire),usize::MAX);
    }
    #[test]
    fn snippet_dispatch_retirement_blocks_queued_read_while_context_permit_is_still_live() {
        let (client, server) = channel();
        let (tx, rx) = mpsc::channel();
        server.attach_wake(Arc::new(move || {
            let _ = tx.send(());
        }));
        discard_attachment_wakes(&rx);
        let permit = Permit::default();
        let revoked = permit.live();
        let dispatch = Arc::new(crate::paste::Dispatch::default());
        let worker_dispatch = dispatch.clone();
        let worker = thread::spawn(move || {
            client.clipboard_for_snippet(None, Arc::new(|| true), revoked, worker_dispatch)
        });
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!dispatch.retire().committed);
        assert!(!permit.0.load(Ordering::Acquire));
        server.drain(|| panic!("retired snippet dispatch must not read clipboard"));
        assert!(worker.join().unwrap().is_err());
    }
    fn submit(
        client: &Client,
        permit: &Permit,
        deadline: Instant,
        live: Live,
    ) -> thread::JoinHandle<Result<Option<String>, String>> {
        let client = client.clone();
        let revoked = permit.live();
        thread::spawn(move || client.clipboard(deadline, live, revoked, Arc::default()))
    }
    #[test]
    fn no_wall_deadline_clipboard_still_retires_with_owner_permit() {
        let (client, server) = channel();
        let (tx, rx) = mpsc::channel();
        client.attach_wake(Arc::new(move || {
            let _ = tx.send(());
        }));
        discard_attachment_wakes(&rx);
        let permit = Permit::default();
        let revoked = permit.live();
        let worker = thread::spawn(move || {
            client.clipboard(None, Arc::new(|| true), revoked, Arc::default())
        });
        rx.recv().unwrap();
        drop(permit);
        server.drain(|| panic!("retired no-deadline capture must not read"));
        assert!(worker.join().unwrap().is_err());
    }
    #[test]
    fn execution_clipboard_keeps_complete_text_above_socket_packet_size() {
        let (client, server) = channel();
        let (tx, rx) = mpsc::channel();
        server.attach_wake(Arc::new(move || {
            let _ = tx.send(());
        }));
        discard_attachment_wakes(&rx);
        let permit = Permit::default();
        let worker = submit(
            &client,
            &permit,
            Instant::now() + Duration::from_secs(5),
            Arc::new(|| true),
        );
        rx.recv().unwrap();
        let text = "Ω".repeat(600_000);
        server.drain(|| Ok(Some(text.clone())));
        assert_eq!(worker.join().unwrap().unwrap(), Some(text));
    }
    #[test]
    fn creation_queue_is_nonblocking_bounded_and_keeps_distinct_replies() {
        let (client, server) = channel();
        let mut pending = vec![];
        let mut permits = vec![];
        for _ in 0..LIMIT {
            let p = Permit::default();
            pending.push(
                client
                    .begin_prefill(
                        Instant::now() + Duration::from_secs(2),
                        Arc::new(|| true),
                        p.live(),
                        Arc::new(|| {}),
                    )
                    .unwrap(),
            );
            permits.push(p);
        }
        let overflow = Permit::default();
        assert!(client
            .begin_prefill(
                Instant::now() + Duration::from_secs(2),
                Arc::new(|| true),
                overflow.live(),
                Arc::new(|| {})
            )
            .is_err());
        let mut n = 0;
        server.drain_with_prefill(
            || panic!("wrong operation"),
            |_| panic!("no execution"),
            || {
                n += 1;
                Ok(Some(n.to_string()))
            },
        );
        for (i, p) in pending.iter().enumerate() {
            assert_eq!(p.take().unwrap().unwrap(), Some((i + 1).to_string()));
        }
    }
    #[test]
    fn creation_revocation_during_native_read_rejects_late_result() {
        let (client, server) = channel();
        let mut permit = Some(Permit::default());
        let live = permit.as_ref().unwrap().live();
        let pending = client
            .begin_prefill(
                Instant::now() + Duration::from_secs(2),
                Arc::new(|| true),
                live,
                Arc::new(|| panic!("revoked completion must not wake")),
            )
            .unwrap();
        server.drain_with_prefill(
            || panic!("wrong operation"),
            |_| panic!("no execution"),
            || {
                drop(permit.take());
                Ok(Some("late own fixture".into()))
            },
        );
        assert!(pending.take().unwrap().is_err());
    }
    #[test]
    fn distinct_replies_and_revocation_before_read() {
        let (client, server) = channel();
        let (tx, rx) = mpsc::channel();
        server.attach_wake(Arc::new(move || {
            let _ = tx.send(());
        }));
        discard_attachment_wakes(&rx);
        let permit = Permit::default();
        let worker = submit(
            &client,
            &permit,
            Instant::now() + Duration::from_secs(1),
            Arc::new(|| true),
        );
        rx.recv().unwrap();
        drop(permit);
        server.drain(|| panic!("Retired request must not read"));
        assert!(worker.join().unwrap().is_err());
        let permit = Permit::default();
        let worker = submit(
            &client,
            &permit,
            Instant::now() + Duration::from_secs(1),
            Arc::new(|| true),
        );
        rx.recv().unwrap();
        server.drain(|| Ok(Some("own fixture".into())));
        assert_eq!(worker.join().unwrap().unwrap(), Some("own fixture".into()));
    }
    #[test]
    fn dispatch_retirement_revokes_queued_capture_before_context_drop() {
        let (client, server) = channel();
        let (tx, rx) = mpsc::channel();
        server.attach_wake(Arc::new(move || {
            let _ = tx.send(());
        }));
        discard_attachment_wakes(&rx);
        let permit = Permit::default();
        let revoked = permit.live();
        let dispatch = Arc::new(crate::workspace::Dispatch::default());
        let authority = dispatch.clone();
        let worker = thread::spawn(move || {
            client.clipboard(
                Instant::now() + Duration::from_secs(1),
                Arc::new(|| true),
                revoked,
                authority,
            )
        });
        rx.recv().unwrap();
        let retired = dispatch.retire();
        assert!(!retired.committed);
        assert!(!permit.0.load(Ordering::Acquire)); // Context still alive during finish I/O.
        server.drain(|| panic!("Retired dispatch must not start a queued read"));
        assert!(worker.join().unwrap().is_err());
    }
    #[test]
    fn deadline_and_disconnect_skip_reads() {
        let (client, server) = channel();
        let (tx, rx) = mpsc::channel();
        server.attach_wake(Arc::new(move || {
            let _ = tx.send(());
        }));
        discard_attachment_wakes(&rx);
        let live = Arc::new(AtomicBool::new(true));
        let check = live.clone();
        let permit = Permit::default();
        let worker = submit(
            &client,
            &permit,
            Instant::now() + Duration::from_secs(1),
            Arc::new(move || check.load(Ordering::Acquire)),
        );
        rx.recv().unwrap();
        live.store(false, Ordering::Release);
        server.drain(|| panic!("Disconnected request must not read"));
        assert!(worker.join().unwrap().is_err());
        assert!(client
            .clipboard(
                Instant::now(),
                Arc::new(|| true),
                permit.live(),
                Arc::default()
            )
            .is_err());
    }
}
