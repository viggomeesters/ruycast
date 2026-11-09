//! Process table and force-quit authority. A PID alone never authorizes kill.
use crate::system_search::{Identity, Ticket, EXPIRED};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub identity: Identity,
    pub name: String,
    pub path: String,
    pub cpu_percent: f64,
    pub memory_bytes: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Cpu,
    Memory,
    Name,
}
pub trait Transport: Send + Sync {
    fn listing(&self, ticket: &Ticket) -> Result<Vec<u8>, String>;
    fn identity(&self, pid: i32) -> Option<Identity>;
    fn path(&self, pid: i32) -> Option<String>;
    fn terminate(&self, pid: i32) -> Result<(), String>;
}
pub struct Native;
impl Transport for Native {
    fn listing(&self, ticket: &Ticket) -> Result<Vec<u8>, String> {
        let mut bytes = vec![];
        crate::os_command::stream(
            "/bin/ps",
            &["-axo", "pid=,pcpu=,rss="],
            ticket,
            2_000_000,
            false,
            |data| {
                bytes.extend_from_slice(data);
                false
            },
        )
        .map_err(|_| "Could not read running processes.")?;
        Ok(bytes)
    }
    fn identity(&self, pid: i32) -> Option<Identity> {
        crate::system_search::native::identity(pid)
    }
    fn path(&self, pid: i32) -> Option<String> {
        let mut bytes = [0u8; 4096];
        let length =
            unsafe { libc::proc_pidpath(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
        if length <= 0 {
            return None;
        }
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
    fn terminate(&self, pid: i32) -> Result<(), String> {
        if unsafe { libc::kill(pid, libc::SIGKILL) } == 0 {
            Ok(())
        } else {
            Err("Could not force quit the process.".into())
        }
    }
}
pub fn table(transport: &dyn Transport, ticket: &Ticket) -> Result<Vec<Resource>, String> {
    let bytes = transport.listing(ticket)?;
    if bytes.len() >= 2_000_000 {
        return Err("Could not read running processes.".into());
    }
    let mut rows = vec![];
    for line in String::from_utf8_lossy(&bytes).lines() {
        ticket.check()?;
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            continue;
        }
        let (Ok(pid), Ok(cpu), Ok(rss)) = (
            fields[0].parse::<i32>(),
            fields[1].parse::<f64>(),
            fields[2].parse::<u64>(),
        ) else {
            continue;
        };
        if pid <= 1
            || pid == unsafe { libc::getpid() }
            || !cpu.is_finite()
            || cpu < 0.
            || rss > u64::MAX / 1024
        {
            continue;
        }
        let Some(identity) = transport
            .identity(pid)
            .filter(|id| id.uid == unsafe { libc::getuid() })
        else {
            continue;
        };
        let Some(path) = transport.path(pid) else {
            continue;
        };
        if transport.identity(pid) != Some(identity) {
            continue;
        }
        let name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        rows.push(Resource {
            identity,
            name,
            path,
            cpu_percent: cpu,
            memory_bytes: rss * 1024,
        });
    }
    ticket.check()?;
    Ok(rows)
}
pub fn matches(resource: &Resource, query: &str) -> bool {
    query.is_empty()
        || resource.identity.pid.to_string().contains(query)
        || objc2::rc::autoreleasepool(|_| {
            objc2_foundation::NSString::from_str(&resource.name)
                .localizedCaseInsensitiveContainsString(&objc2_foundation::NSString::from_str(
                    query,
                ))
        })
}
pub fn compare(a: &Resource, b: &Resource, order: Sort) -> std::cmp::Ordering {
    let primary = match order {
        Sort::Cpu => b.cpu_percent.total_cmp(&a.cpu_percent),
        Sort::Memory => b.memory_bytes.cmp(&a.memory_bytes),
        Sort::Name => std::cmp::Ordering::Equal,
    };
    primary.then_with(|| {
        if a.name == b.name {
            a.identity.pid.cmp(&b.identity.pid)
        } else {
            objc2::rc::autoreleasepool(|_| {
                let order = objc2_foundation::NSString::from_str(&a.name)
                    .localizedStandardCompare(&objc2_foundation::NSString::from_str(&b.name));
                (order as isize).cmp(&0)
            })
        }
    })
}
pub fn sort(rows: &mut [Resource], order: Sort) {
    rows.sort_by(|a, b| compare(a, b, order));
}
pub fn terminate(
    transport: &dyn Transport,
    identity: Identity,
    ticket: &Ticket,
) -> Result<(), String> {
    ticket.check()?;
    if identity.uid != unsafe { libc::getuid() }
        || identity.pid <= 1
        || identity.pid == unsafe { libc::getpid() }
    {
        return Err("This process cannot be force quit.".into());
    }
    if transport.identity(identity.pid) != Some(identity) {
        return Err(EXPIRED.into());
    }
    ticket.check()?;
    transport.terminate(identity.pid)
}
#[derive(Default)]
pub struct Cache {
    sampled: Option<Instant>,
    rows: Vec<Resource>,
}
impl Cache {
    pub fn read(
        &mut self,
        transport: &dyn Transport,
        query: &str,
        ticket: &Ticket,
    ) -> Result<Vec<Resource>, String> {
        if self
            .sampled
            .is_none_or(|at| at.elapsed() > Duration::from_millis(1500))
        {
            let rows = table(transport, ticket)?;
            self.rows = rows;
            self.sampled = Some(Instant::now());
        }
        ticket.check()?;
        let mut rows = self
            .rows
            .iter()
            .filter(|r| matches(r, query))
            .cloned()
            .collect::<Vec<_>>();
        sort(&mut rows, Sort::Cpu);
        rows.truncate(1000);
        Ok(rows)
    }
    pub fn invalidate(&mut self) {
        self.sampled = None;
    }
}
#[cfg(any(test, feature = "test-tools"))]
pub mod fixture {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    };
    pub struct Fixture {
        pub rows: Mutex<Vec<Resource>>,
        pub kills: Mutex<Vec<i32>>,
        pub stale_after_path: AtomicBool,
    }
    impl Default for Fixture {
        fn default() -> Self {
            Self {
                rows: Mutex::new(vec![
                    Resource {
                        identity: Identity {
                            pid: 730011,
                            uid: unsafe { libc::getuid() },
                            seconds: 101,
                            micros: 33,
                        },
                        name: "Owned Process 2".into(),
                        path: "/owned/Owned Process 2".into(),
                        cpu_percent: 20.,
                        memory_bytes: 4096,
                    },
                    Resource {
                        identity: Identity {
                            pid: 730012,
                            uid: unsafe { libc::getuid() },
                            seconds: 102,
                            micros: 44,
                        },
                        name: "Owned Process 10".into(),
                        path: "/owned/Owned Process 10".into(),
                        cpu_percent: 10.,
                        memory_bytes: 8192,
                    },
                ]),
                kills: Default::default(),
                stale_after_path: AtomicBool::new(false),
            }
        }
    }
    impl Transport for Fixture {
        fn listing(&self, _: &Ticket) -> Result<Vec<u8>, String> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .map(|r| {
                    format!(
                        "{} {} {}\n",
                        r.identity.pid,
                        r.cpu_percent,
                        r.memory_bytes / 1024
                    )
                })
                .collect::<String>()
                .into_bytes())
        }
        fn identity(&self, pid: i32) -> Option<Identity> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.identity.pid == pid)
                .map(|r| r.identity)
        }
        fn path(&self, pid: i32) -> Option<String> {
            let mut rows = self.rows.lock().unwrap();
            let row = rows.iter_mut().find(|r| r.identity.pid == pid)?;
            let path = row.path.clone();
            if self.stale_after_path.load(Ordering::Acquire) {
                row.identity.seconds += 1;
            }
            Some(path)
        }
        fn terminate(&self, pid: i32) -> Result<(), String> {
            self.kills.lock().unwrap().push(pid);
            self.rows.lock().unwrap().retain(|r| r.identity.pid != pid);
            Ok(())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn ticket() -> Ticket {
        Ticket {
            deadline: Instant::now() + Duration::from_secs(2),
            live: Arc::new(|| true),
        }
    }
    #[test]
    fn owned_table_sort_and_immutable_kill_identity() {
        let f = fixture::Fixture::default();
        let mut rows = table(&f, &ticket()).unwrap();
        assert_eq!(rows.len(), 2);
        sort(&mut rows, Sort::Memory);
        assert_eq!(rows[0].identity.pid, 730012);
        sort(&mut rows, Sort::Name);
        assert_eq!(rows[0].name, "Owned Process 2");
        let old = rows[0].identity;
        f.rows.lock().unwrap()[0].identity.seconds += 1;
        assert!(terminate(&f, old, &ticket()).is_err());
        assert!(f.kills.lock().unwrap().is_empty());
        let current = f.rows.lock().unwrap()[0].identity;
        terminate(&f, current, &ticket()).unwrap();
        assert_eq!(*f.kills.lock().unwrap(), vec![730011]);
    }
    #[test]
    fn matching_self_and_foreign_uid_identities_refuse_before_termination() {
        let owner = unsafe { libc::getuid() };
        for identity in [
            Identity { pid: unsafe { libc::getpid() }, uid: owner, seconds: 101, micros: 33 },
            Identity { pid: 730011, uid: owner.wrapping_add(1), seconds: 101, micros: 33 },
        ] {
            let fixture = fixture::Fixture::default();
            fixture.rows.lock().unwrap()[0].identity = identity;
            assert_eq!(fixture.identity(identity.pid), Some(identity), "identity mismatch must not mask the authority guard");
            assert_eq!(terminate(&fixture, identity, &ticket()).unwrap_err(), "This process cannot be force quit.");
            assert!(fixture.kills.lock().unwrap().is_empty());
            assert_eq!(fixture.identity(identity.pid), Some(identity));
        }
    }
    #[test]
    fn replacement_during_path_lookup_drops_rows() {
        let f = fixture::Fixture::default();
        f.stale_after_path
            .store(true, std::sync::atomic::Ordering::Release);
        assert!(table(&f, &ticket()).unwrap().is_empty());
    }
}
