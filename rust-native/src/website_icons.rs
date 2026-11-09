//! Origin-only favicon transport. Cookies, credentials and URL caches are off;
//! response headers and streamed chunks are bounded before image decoding.
use objc2::{
    define_class, msg_send,
    rc::{autoreleasepool, Retained},
    runtime::ProtocolObject,
    AnyThread, DefinedClass,
};
use objc2_foundation::*;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex,
    },
    time::{Duration, Instant},
};
const MAX: usize = 1_048_576;
#[derive(Default)]
struct Download {
    bytes: Vec<u8>,
    valid: bool,
    complete: bool,
}
#[derive(Default)]
struct State {
    download: Mutex<Download>,
    done: Condvar,
}
define_class!(
    #[unsafe(super=NSObject)]
    #[thread_kind=AnyThread]
    #[ivars=State]
    struct Receiver;
    unsafe impl NSObjectProtocol for Receiver {}
    unsafe impl NSURLSessionDelegate for Receiver {}
    unsafe impl NSURLSessionTaskDelegate for Receiver {
        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn complete(&self, _: &NSURLSession, _: &NSURLSessionTask, error: Option<&NSError>) {
            let mut s = self.ivars().download.lock().unwrap();
            if error.is_some() {
                s.valid = false;
            }
            s.complete = true;
            self.ivars().done.notify_all();
        }
        #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
        unsafe fn redirect(
            &self,
            _: &NSURLSession,
            _: &NSURLSessionTask,
            _: &NSHTTPURLResponse,
            request: &NSURLRequest,
            handler: &block2::DynBlock<dyn Fn(*mut NSURLRequest)>,
        ) {
            let valid = request.URL().is_some_and(|u| safe_url(&u));
            handler.call((if valid {
                (request as *const NSURLRequest).cast_mut()
            } else {
                std::ptr::null_mut()
            },));
        }
    }
    unsafe impl NSURLSessionDataDelegate for Receiver {
        #[unsafe(method(URLSession:dataTask:didReceiveResponse:completionHandler:))]
        unsafe fn response(
            &self,
            _: &NSURLSession,
            _: &NSURLSessionDataTask,
            response: &NSURLResponse,
            handler: &block2::DynBlock<dyn Fn(NSURLSessionResponseDisposition)>,
        ) {
            let valid = response
                .downcast_ref::<NSHTTPURLResponse>()
                .is_some_and(|r| (200..300).contains(&r.statusCode()))
                && response.expectedContentLength() <= MAX as i64;
            self.ivars().download.lock().unwrap().valid = valid;
            handler.call((if valid {
                NSURLSessionResponseDisposition::Allow
            } else {
                NSURLSessionResponseDisposition::Cancel
            },));
        }
        #[unsafe(method(URLSession:dataTask:didReceiveData:))]
        fn data(&self, _: &NSURLSession, task: &NSURLSessionDataTask, data: &NSData) {
            let mut s = self.ivars().download.lock().unwrap();
            if s.bytes.len().saturating_add(data.length()) > MAX {
                s.valid = false;
                drop(s);
                task.cancel();
            } else {
                s.bytes
                    .extend_from_slice(unsafe { data.as_bytes_unchecked() });
            }
        }
    }
);
fn safe_url(url: &NSURL) -> bool {
    url.scheme()
        .is_some_and(|s| ["http", "https"].contains(&s.to_string().to_lowercase().as_str()))
        && url.user().is_none()
        && url.password().is_none()
}
pub fn get(raw: &str, stop: &AtomicBool) -> Option<Vec<u8>> {
    autoreleasepool(|_| {
        if stop.load(Ordering::Acquire) {
            return None;
        }
        let url = NSURL::URLWithString(&NSString::from_str(raw))?;
        if !safe_url(&url) {
            return None;
        }
        let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
        config.setHTTPShouldSetCookies(false);
        config.setHTTPCookieStorage(None);
        config.setURLCredentialStorage(None);
        config.setURLCache(None);
        config.setTimeoutIntervalForRequest(3.);
        config.setTimeoutIntervalForResource(4.);
        let receiver = Receiver::alloc().set_ivars(State::default());
        let receiver: Retained<Receiver> = unsafe { msg_send![super(receiver), init] };
        let queue = NSOperationQueue::new();
        queue.setMaxConcurrentOperationCount(1);
        let session = unsafe {
            NSURLSession::sessionWithConfiguration_delegate_delegateQueue(
                &config,
                Some(ProtocolObject::from_ref(&*receiver)),
                Some(&queue),
            )
        };
        let request = NSMutableURLRequest::requestWithURL(&url);
        request.setValue_forHTTPHeaderField(
            Some(&NSString::from_str("Ruycast/0.3 website icon")),
            &NSString::from_str("User-Agent"),
        );
        session.dataTaskWithRequest(&request).resume();
        let end = Instant::now() + Duration::from_secs(5);
        let mut s = receiver.ivars().download.lock().unwrap();
        while !s.complete && !stop.load(Ordering::Acquire) && Instant::now() < end {
            s = receiver
                .ivars()
                .done
                .wait_timeout(s, Duration::from_millis(20))
                .unwrap()
                .0;
        }
        let result = (s.complete && s.valid && !stop.load(Ordering::Acquire))
            .then(|| std::mem::take(&mut s.bytes));
        drop(s);
        session.invalidateAndCancel();
        result
    })
}
pub fn candidates(html: &str, origin: &str) -> Vec<String> {
    // Foundation's source regex contract, including quoted/unquoted attributes.
    let links = NSRegularExpression::initWithPattern_options_error(
        NSRegularExpression::alloc(),
        &NSString::from_str("<link\\b[^>]*>"),
        NSRegularExpressionOptions::CaseInsensitive,
    )
    .ok();
    let attrs = NSRegularExpression::initWithPattern_options_error(
        NSRegularExpression::alloc(),
        &NSString::from_str(r#"([\w-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#),
        NSRegularExpressionOptions::CaseInsensitive,
    )
    .ok();
    let (Some(links), Some(attrs), Some(base)) = (
        links,
        attrs,
        NSURL::URLWithString(&NSString::from_str(origin)),
    ) else {
        return vec![];
    };
    let text = NSString::from_str(html);
    let mut out = vec![];
    for m in links.matchesInString_options_range(
        &text,
        NSMatchingOptions::empty(),
        NSRange::new(0, text.length()),
    ) {
        let tag = text.substringWithRange(m.range());
        let mut values = std::collections::HashMap::new();
        for a in attrs.matchesInString_options_range(
            &tag,
            NSMatchingOptions::empty(),
            NSRange::new(0, tag.length()),
        ) {
            let key = tag
                .substringWithRange(a.rangeAtIndex(1))
                .to_string()
                .to_lowercase();
            let range = (2..=4)
                .map(|i| a.rangeAtIndex(i))
                .find(|r| r.location != NSNotFound as usize);
            if let Some(r) = range {
                values.insert(key, tag.substringWithRange(r).to_string());
            }
        }
        let rel = values
            .get("rel")
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        if !rel
            .split_whitespace()
            .any(|s| ["icon", "apple-touch-icon"].contains(&s))
        {
            continue;
        }
        if let Some(url) = values
            .get("href")
            .and_then(|s| {
                NSURL::URLWithString_relativeToURL(
                    &NSString::from_str(&s.replace("&amp;", "&")),
                    Some(&base),
                )
            })
            .and_then(|u| u.absoluteURL())
            .filter(|u| safe_url(u))
            .and_then(|u| u.absoluteString())
        {
            let value = url.to_string();
            if !out.contains(&value) {
                out.push(value);
            }
            if out.len() == 6 {
                break;
            }
        }
    }
    out
}

use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread::JoinHandle,
};
type Wake = Arc<dyn Fn() + Send + Sync>;
struct ResultImage {
    key: String,
    png: Option<Arc<Vec<u8>>>,
    finished: bool,
}
pub struct Service {
    sender: Option<mpsc::SyncSender<String>>,
    receiver: mpsc::Receiver<ResultImage>,
    pending: HashSet<String>,
    retry: HashMap<String, Instant>,
    images: HashMap<String, Arc<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}
impl Service {
    pub fn new(cache: PathBuf, wake: Wake) -> Result<Self, String> {
        let (tx, rx) = mpsc::sync_channel::<String>(64);
        let receiver = Arc::new(Mutex::new(rx));
        let (result_tx, result_rx) = mpsc::sync_channel(256);
        let stop = Arc::new(AtomicBool::new(false));
        // Serialize only disk publication. The shared writer's staging nonce is
        // wall-clock based; simultaneous completions can otherwise collide.
        let publication = Arc::new(Mutex::new(()));
        let mut threads = vec![];
        for i in 0..4 {
            let receiver = receiver.clone();
            let tx = result_tx.clone();
            let cache = cache.clone();
            let wake = wake.clone();
            let stop = stop.clone();
            let publication = publication.clone();
            let worker = std::thread::Builder::new()
                .name(format!("ruycast-website-icon-{i}"))
                .spawn(move || loop {
                    let next = { receiver.lock().unwrap().recv() };
                    let Ok(origin) = next else {
                        break;
                    };
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let key = key(&origin);
                    let file = cache.join(format!("{key}.png"));
                    let miss = cache.join(format!("{key}.miss"));
                    let mut png = read_cached(&file).map(Arc::new);
                    if let Some(png) = &png {
                        let _ = tx.try_send(ResultImage {
                            key: key.clone(),
                            png: Some(png.clone()),
                            finished: false,
                        });
                        wake();
                    }
                    if age(&file) > Duration::from_secs(7 * 86400)
                        && age(&miss) > Duration::from_secs(3600)
                    {
                        if let Some(fresh) = fetch(&origin, &stop) {
                            png = Some(Arc::new(fresh.clone()));
                            if !stop.load(Ordering::Acquire) {
                                let _commit = publication.lock().unwrap();
                                if stop.load(Ordering::Acquire) {
                                    break;
                                }
                                let _ = fs::DirBuilder::new()
                                    .recursive(true)
                                    .mode(0o700)
                                    .create(&cache);
                                let _ = crate::private_storage::write(&file, &fresh, true);
                                let _ = fs::remove_file(&miss);
                            }
                        } else if !stop.load(Ordering::Acquire) {
                            let _commit = publication.lock().unwrap();
                            if stop.load(Ordering::Acquire) {
                                break;
                            }
                            let _ = fs::DirBuilder::new()
                                .recursive(true)
                                .mode(0o700)
                                .create(&cache);
                            let _ = crate::private_storage::write(&miss, &[], true);
                        }
                        if !stop.load(Ordering::Acquire) {
                            let _commit = publication.lock().unwrap();
                            prune(&cache);
                        }
                    }
                    let _ = tx.try_send(ResultImage {
                        key,
                        png,
                        finished: true,
                    });
                    wake();
                })
                .map_err(|e| e.to_string())?;
            threads.push(worker);
        }
        Ok(Self {
            sender: Some(tx),
            receiver: result_rx,
            pending: HashSet::new(),
            retry: HashMap::new(),
            images: HashMap::new(),
            stop,
            threads,
        })
    }
    pub fn image(&mut self, origin: &str) -> Option<Arc<Vec<u8>>> {
        if crate::presentation::website_origin(origin).as_deref() != Some(origin) {
            return None;
        }
        let key = key(origin);
        if let Some(png) = self.images.get(&key) {
            return Some(png.clone());
        }
        if self.pending.len() < 64
            && !self.pending.contains(&key)
            && self
                .retry
                .get(&key)
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(3600))
            && self
                .sender
                .as_ref()
                .is_some_and(|tx| tx.try_send(origin.into()).is_ok())
        {
            self.pending.insert(key);
        }
        None
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(result) = self.receiver.try_recv() {
            changed = true;
            if result.finished {
                self.pending.remove(&result.key);
                self.retry.insert(result.key.clone(), Instant::now());
                if self.retry.len() > 1024 {
                    if let Some(oldest) = self
                        .retry
                        .iter()
                        .min_by_key(|(_, at)| **at)
                        .map(|(k, _)| k.clone())
                    {
                        self.retry.remove(&oldest);
                    }
                }
            }
            if let Some(png) = result.png {
                if self.images.len() >= 512 {
                    if let Some(key) = self.images.keys().next().cloned() {
                        self.images.remove(&key);
                    }
                }
                self.images.insert(result.key, png);
            }
        }
        changed
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.sender.take();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
fn key(origin: &str) -> String {
    format!("{:x}", Sha256::digest(origin.as_bytes()))
}
fn age(path: &Path) -> Duration {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|d| d.elapsed().ok())
        .unwrap_or(Duration::MAX)
}
fn read_cached(path: &Path) -> Option<Vec<u8>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > 131072 {
        return None;
    }
    let mut bytes = vec![];
    file.take(131073).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 131072 {
        return None;
    }
    crate::clipboard_image::website_icon(&bytes).ok()
}
fn fetch(origin: &str, stop: &AtomicBool) -> Option<Vec<u8>> {
    let icon =
        |url: &str| get(url, stop).and_then(|b| crate::clipboard_image::website_icon(&b).ok());
    if let Some(bytes) = icon(&format!("{origin}favicon.ico")) {
        return Some(bytes);
    }
    if let Some(html) = get(origin, stop).and_then(|b| String::from_utf8(b).ok()) {
        for candidate in candidates(&html, origin) {
            if let Some(bytes) = icon(&candidate) {
                return Some(bytes);
            }
        }
    }
    for name in ["apple-touch-icon.png", "favicon.png"] {
        if let Some(bytes) = icon(&format!("{origin}{name}")) {
            return Some(bytes);
        }
    }
    None
}
fn prune(cache: &Path) {
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    let mut entries = entries
        .flatten()
        .filter(|e| {
            let p = e.path();
            p.extension().is_some_and(|s| s == "png" || s == "miss")
                && p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        })
        .collect::<Vec<_>>();
    if entries.len() > 1024 {
        entries.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
        let excess = entries.len() - 1024;
        for entry in entries.into_iter().take(excess) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        os::unix::fs::PermissionsExt,
        sync::atomic::AtomicUsize,
    };
    #[test]
    fn cache_special_files_and_close_are_bounded() {
        const CHILD: &str = "RUYCAST_ICON_CACHE_REPAIR_CHILD";
        if let Some(base) = std::env::var_os(CHILD) {
            let base = PathBuf::from(base);
            let origin = "http://127.0.0.1:1/";
            let file = base.join(format!("{}.png", key(origin)));
            for kind in [
                "fifo",
                "symlink",
                "directory",
                "oversize",
                "invalid",
                "valid",
            ] {
                match kind {
                    "fifo" => {
                        use std::os::unix::ffi::OsStrExt;
                        let path = std::ffi::CString::new(file.as_os_str().as_bytes()).unwrap();
                        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
                    }
                    "symlink" => std::os::unix::fs::symlink(base.join("valid.png"), &file).unwrap(),
                    "directory" => fs::create_dir(&file).unwrap(),
                    "oversize" => fs::write(&file, vec![0; 131073]).unwrap(),
                    "invalid" => fs::write(&file, b"invalid").unwrap(),
                    "valid" => fs::write(&file, png()).unwrap(),
                    _ => unreachable!(),
                }
                let start = Instant::now();
                assert_eq!(read_cached(&file).is_some(), kind == "valid", "{kind}");
                let mut service = Service::new(base.clone(), Arc::new(|| {})).unwrap();
                service.image(origin);
                std::thread::sleep(Duration::from_millis(30));
                drop(service);
                assert!(start.elapsed() < Duration::from_millis(500), "{kind}");
                if kind == "directory" {
                    fs::remove_dir(&file).unwrap();
                } else {
                    fs::remove_file(&file).unwrap();
                }
            }
            return;
        }
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Owned fixture parent required"),
        );
        let base = parent.join(format!("favicon-special-{}", std::process::id()));
        fs::create_dir(&base).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(base.clone());
        fs::write(base.join("valid.png"), png()).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "website_icons::tests::cache_special_files_and_close_are_bounded",
                "--nocapture",
            ])
            .env(CHILD, &base)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Cache child exceeded closure deadline (killed and reaped)");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn server(responses: Vec<Vec<u8>>) -> (String, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = vec![];
            for response in responses {
                let end = Instant::now() + Duration::from_secs(6);
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(Instant::now() < end, "Local fixture was never requested");
                    std::thread::sleep(Duration::from_millis(5));
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    request.push_str(&line);
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                requests.push(request);
                let _ = stream.write_all(&response);
            }
            requests
        });
        (url, handle)
    }
    fn response(status: &str, body: &[u8]) -> Vec<u8> {
        let mut r=format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nSet-Cookie: private=forbidden\r\nConnection: close\r\n\r\n",body.len()).into_bytes();
        r.extend_from_slice(body);
        r
    }
    fn png() -> Vec<u8> {
        vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2,
            8, 6, 0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 104, 112,
            80, 248, 15, 194, 12, 48, 6, 0, 64, 180, 7, 125, 193, 185, 247, 76, 0, 0, 0, 0, 73, 69,
            78, 68, 174, 66, 96, 130,
        ]
    }
    #[test]
    fn origin_and_candidate_privacy() {
        autoreleasepool(|_| {
            assert!(crate::clipboard_image::website_icon(&vec![0; 1_048_577]).is_err());
            assert!(crate::clipboard_image::website_icon(b"not an image").is_err());
            assert_eq!(
                crate::presentation::website_origin(
                    "HTTPS://user:secret@Example.COM:8443/private?p=secret#private"
                ),
                Some("https://example.com:8443/".into())
            );
            assert_eq!(crate::presentation::website_origin("file:///secret"), None);
            assert_eq!(crate::presentation::website_origin("https://{host}/"), None);
            assert_eq!(
                candidates(
                    r#"<LINK rel='shortcut icon' href='/one?a=1&amp;b=2'><link rel=apple-touch-icon href=two.png><link rel='icon' href='file:///private'><link rel=icon href='https://u:p@example.test/private'><link rel=icon href='/one?a=1&amp;b=2'>"#,
                    "https://example.test/"
                ),
                vec![
                    "https://example.test/one?a=1&b=2",
                    "https://example.test/two.png"
                ]
            );
        });
    }
    #[test]
    fn actual_transport_stream_bounds_status_cookies_and_redirects() {
        let stop = AtomicBool::new(false);
        let (url, server) = server(vec![
            response("200 OK", b"owned"),
            response("404 Not Found", b"bad"),
        ]);
        assert_eq!(get(&format!("{url}one"), &stop), Some(b"owned".to_vec()));
        assert_eq!(get(&format!("{url}two"), &stop), None);
        let requests = server.join().unwrap();
        assert!(requests
            .iter()
            .all(|s| !s.to_lowercase().contains("\r\ncookie:")
                && !s.to_lowercase().contains("authorization:")));
        assert!(requests[0].contains("User-Agent: Ruycast/0.3 website icon"));
        let (url, server) = self::server(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_vec(),
        ]);
        assert!(get(&url, &stop).is_none());
        server.join().unwrap();
        let body = vec![0u8; 600000];
        let mut chunked =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        for _ in 0..2 {
            chunked.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
            chunked.extend_from_slice(&body);
            chunked.extend_from_slice(b"\r\n");
        }
        chunked.extend_from_slice(b"0\r\n\r\n");
        let (url, server) = self::server(vec![chunked]);
        assert!(get(&url, &stop).is_none());
        server.join().unwrap();
        let(url,server)=self::server(vec![b"HTTP/1.1 302 Found\r\nLocation: http://user:secret@127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()]);
        assert!(get(&url, &stop).is_none());
        server.join().unwrap();
        assert!(get("file:///personal", &stop).is_none());
    }
    #[test]
    fn four_workers_cache_publication_and_cancel_join() {
        let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
            .map(PathBuf::from)
            .expect("Owned fixture parent required");
        let cache = parent.join(format!("favicon-{}", std::process::id()));
        fs::create_dir(&cache).unwrap();
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let mut servers = vec![];
        let mut origins = vec![];
        for _ in 0..4 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            origins.push(format!("http://{}/", listener.local_addr().unwrap()));
            let entered = entered.clone();
            let release = release.clone();
            servers.push(std::thread::spawn(move || {
                let end = Instant::now() + Duration::from_secs(6);
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(Instant::now() < end);
                    std::thread::sleep(Duration::from_millis(5));
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                entered.fetch_add(1, Ordering::Release);
                while !release.load(Ordering::Acquire) && Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(5));
                }
                let _ = stream.write_all(&response("200 OK", &png()));
            }));
        }
        let mut service = Service::new(cache.clone(), Arc::new(|| {})).unwrap();
        let start = Instant::now();
        assert!(service.image(&origins[0]).is_none());
        assert!(service.image(&origins[0]).is_none());
        assert!(
            start.elapsed() < Duration::from_millis(40),
            "Two icon requests blocked on their pending fetch"
        );
        assert_eq!(service.pending.len(), 1);
        for origin in &origins[1..] {
            assert!(service.image(origin).is_none());
        }
        let end = Instant::now() + Duration::from_secs(5);
        while entered.load(Ordering::Acquire) < 4 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        let simultaneous = entered.load(Ordering::Acquire);
        release.store(true, Ordering::Release);
        assert_eq!(simultaneous, 4, "Worker receiver mutex serialized fetches");
        for server in servers {
            server.join().unwrap();
        }
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            service.poll();
            if service.pending.is_empty() {
                break;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        for origin in &origins {
            assert!(service.image(origin).is_some());
            let file = cache.join(format!("{}.png", key(origin)));
            assert!(file.is_file());
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(service);
        let mut service = Service::new(cache.clone(), Arc::new(|| {})).unwrap();
        for origin in &origins {
            service.image(origin);
        }
        let end = Instant::now() + Duration::from_secs(2);
        while !service.pending.is_empty() {
            service.poll();
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        for origin in &origins {
            assert!(service.image(origin).is_some());
        }
        drop(service);
        fs::remove_dir_all(cache).unwrap();
    }
    #[test]
    fn two_thousand_origins_bound_admission_and_deduplicate_while_four_workers_wait() {
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Owned fixture parent required"),
        );
        let cache = parent.join(format!("favicon-flood-{}", std::process::id()));
        fs::create_dir(&cache).unwrap();
        // Every offered origin is our own retained loopback listener. Even a
        // broken admission limit cannot contact another service or the network.
        let mut listeners = Vec::new();
        let mut origins = Vec::new();
        for _ in 0..2000 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            origins.push(format!("http://{}/", listener.local_addr().unwrap()));
            listeners.push(listener);
        }
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let mut servers = Vec::new();
        for listener in listeners.drain(..4) {
            let entered = entered.clone();
            let release = release.clone();
            servers.push(std::thread::spawn(move || {
                let end = Instant::now() + Duration::from_secs(6);
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(Instant::now() < end);
                    std::thread::sleep(Duration::from_millis(5));
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                entered.fetch_add(1, Ordering::Release);
                while !release.load(Ordering::Acquire) && Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(5));
                }
                let _ = stream.write_all(&response("200 OK", &png()));
            }));
        }
        let mut service = Service::new(cache.clone(), Arc::new(|| {})).unwrap();
        for origin in &origins[..4] {
            assert!(service.image(origin).is_none());
        }
        let end = Instant::now() + Duration::from_secs(3);
        while entered.load(Ordering::Acquire) < 4 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(entered.load(Ordering::Acquire), 4);
        assert_eq!(service.threads.len(), 4);
        for origin in &origins[4..] {
            assert!(service.image(origin).is_none());
            assert!(service.pending.len() <= 64);
        }
        assert_eq!(service.pending.len(), 64);
        for origin in &origins[..64] {
            assert!(service.pending.contains(&key(origin)));
        }
        for origin in &origins[64..] {
            assert!(!service.pending.contains(&key(origin)));
        }
        let pending = service.pending.clone();
        for _ in 0..1000 {
            assert!(service.image(&origins[0]).is_none());
        }
        assert_eq!(service.pending, pending);
        // Inspect the actual bounded sender without releasing any worker: sixty
        // admitted jobs leave exactly four slots. Duplicate requests must not
        // have consumed those slots even though pending is a HashSet.
        let sender = service.sender.as_ref().unwrap();
        for origin in &origins[64..68] {
            sender.try_send(origin.clone()).unwrap();
        }
        assert!(matches!(
            sender.try_send(origins[68].clone()),
            Err(mpsc::TrySendError::Full(_))
        ));
        assert!(service.images.is_empty() && service.retry.is_empty());
        // Four live requests stay blocked; no fifth fetch starts, including the
        // four extra sender-capacity observations above.
        for listener in &listeners {
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
        drop(service); // Cancel/join before releasing servers or listener owners.
        release.store(true, Ordering::Release);
        for server in servers {
            server.join().unwrap();
        }
        for listener in &listeners {
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);
        drop(listeners);
        fs::remove_dir(cache).unwrap();
    }
    #[test]
    fn active_download_close_cancels_and_joins_without_cache_publication() {
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Owned fixture parent required"),
        );
        let cache = parent.join(format!("favicon-close-{}", std::process::id()));
        fs::create_dir(&cache).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let (entered, ready) = mpsc::sync_channel(1);
        let (release, done) = mpsc::sync_channel::<()>(1);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            entered.send(()).unwrap();
            let _ = done.recv_timeout(Duration::from_secs(3));
            let _ = stream.write_all(&response("200 OK", &png()));
        });
        let mut service = Service::new(cache.clone(), Arc::new(|| {})).unwrap();
        service.image(&origin);
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        let start = Instant::now();
        drop(service);
        let elapsed = start.elapsed();
        release.send(()).unwrap();
        server.join().unwrap();
        assert!(
            elapsed < Duration::from_millis(500),
            "Close waited for transport deadline: {elapsed:?}"
        );
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);
        fs::remove_dir(cache).unwrap();
    }
}
