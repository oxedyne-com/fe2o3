//! The kernel cuts a command name to fifteen bytes (`TASK_COMM_LEN` less its
//! terminator), so a resident named longer than that is matched on its first
//! fifteen. `daimond_gateway` fits exactly; a longer name compared whole would
//! never match anything, and would read as a service that is not running.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::PROC_ROOT;

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fs,
    path::Path,
};

pub const CGROUP_ROOT:  &str = "/sys/fs/cgroup";
pub const COMM_MAX:     usize = 15;     // TASK_COMM_LEN less its terminator

/// The memory held by one named service: every process with that command name,
/// summed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Resident {
    pub name:       String,
    pub procs:      u32,            // processes matched
    pub rss_kib:    u64,            // summed `VmRSS`, which the kernel labels kB
    // The tighter of `memory.high` and `memory.max` on the cgroup the matched
    // processes share. `None` when neither is set, or when the processes do not
    // share one cgroup, since one cap over several services is not this one's.
    pub cap_kib:    Option<u64>,
}

impl Resident {
    /// Per cent of the cap in use, rounded down.
    pub fn cap_pct(&self) -> Option<u64> {
        match self.cap_kib {
            Some(cap) if cap > 0 => Some(self.rss_kib.saturating_mul(100) / cap),
            _ => None,
        }
    }

    /// Reads every process named in `names` in one walk of `/proc`, and each
    /// one's cgroup cap from `/sys/fs/cgroup`. A name nothing matches comes back
    /// with no processes rather than as an error: a service that is not running
    /// is the reading, not a failure to take one.
    pub fn sample(names: &[String]) -> Outcome<Vec<Self>> {
        Self::sample_under(names, Path::new(PROC_ROOT), Path::new(CGROUP_ROOT))
    }

    /// [`Self::sample`] against another process and cgroup root, so the walk can
    /// be exercised on a tree built for the purpose.
    pub fn sample_under(
        names:          &[String],
        proc_root:      &Path,
        cgroup_root:    &Path,
    )
        -> Outcome<Vec<Self>>
    {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        // Each resident with the cgroup path of every process it matched.
        let mut found: Vec<(Self, Vec<String>)> = names.iter()
            .map(|n| (Self { name: n.clone(), ..Self::default() }, Vec::new()))
            .collect();
        let dir = match fs::read_dir(proc_root) {
            Ok(d) => d,
            Err(e) => return Err(err!(e,
                "Listing {:?} to find resident processes.", proc_root;
                IO, File, Read)),
        };
        for entry in dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let file_name = entry.file_name();
            let pid = match file_name.to_str() {
                Some(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => s,
                _ => continue,
            };
            // A process can exit between the listing and the read. That is a
            // race with the scheduler, not a fault in the reading.
            let status = match fs::read_to_string(proc_root.join(pid).join("status")) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let (comm, rss) = match status_name_rss(&status) {
                (Some(c), Some(r)) => (c, r),
                // No `VmRSS` is a kernel thread, which holds no user memory.
                _ => continue,
            };
            let mut cgroup: Option<String> = None;
            for (res, paths) in found.iter_mut() {
                if !comm_matches(comm, &res.name) {
                    continue;
                }
                res.procs = res.procs.saturating_add(1);
                res.rss_kib = res.rss_kib.saturating_add(rss);
                if cgroup.is_none() {
                    // Unreadable is recorded as its own path, so it can never be
                    // mistaken for agreement with the other processes.
                    cgroup = Some(
                        match fs::read_to_string(proc_root.join(pid).join("cgroup")) {
                            Ok(c) => match cgroup_v2_path(&c) {
                                Some(p) => p.to_string(),
                                None    => fmt!("?{}", pid),
                            },
                            Err(_) => fmt!("?{}", pid),
                        });
                }
                if let Some(p) = &cgroup {
                    paths.push(p.clone());
                }
            }
        }
        let mut out = Vec::with_capacity(found.len());
        for (mut res, mut paths) in found {
            paths.sort();
            paths.dedup();
            if paths.len() == 1 && !paths[0].starts_with('?') {
                res.cap_kib = cgroup_cap_kib(cgroup_root, &paths[0]);
            }
            out.push(res);
        }
        Ok(out)
    }
}

/// Does a kernel command name belong to the resident called `want`?
pub fn comm_matches(comm: &str, want: &str) -> bool {
    let want = want.as_bytes();
    let want = if want.len() > COMM_MAX { &want[..COMM_MAX] } else { want };
    comm.as_bytes() == want
}

/// The `Name` and the `VmRSS` in kibibytes from a `/proc/<pid>/status` body.
pub fn status_name_rss(content: &str) -> (Option<&str>, Option<u64>) {
    let mut name = None;
    let mut rss = None;
    for line in content.lines() {
        let (key, rest) = match line.split_once(':') {
            Some(kv) => kv,
            None => continue,
        };
        match key {
            "Name"  => name = Some(rest.trim()),
            "VmRSS" => rss = rest.split_whitespace().next()
                .and_then(|t| t.parse::<u64>().ok()),
            _ => (),
        }
        if name.is_some() && rss.is_some() {
            break;
        }
    }
    (name, rss)
}

/// The unified-hierarchy path from a `/proc/<pid>/cgroup` body, the line that
/// begins `0::`. A host on the legacy hierarchy alone has no such line.
pub fn cgroup_v2_path(content: &str) -> Option<&str> {
    content.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(|p| p.trim())
}

/// A `memory.max` or `memory.high` body in kibibytes, or `None` for `max`,
/// which is the kernel's word for no limit.
pub fn cgroup_limit_kib(content: &str) -> Option<u64> {
    content.trim().parse::<u64>().ok().map(|bytes| bytes / 1024)
}

/// The tighter of the two limits the kernel enforces on a cgroup. `memory.high`
/// throttles and reclaims, `memory.max` kills; either is where the service
/// stops having the memory it asked for.
fn cgroup_cap_kib(cgroup_root: &Path, path: &str) -> Option<u64> {
    // `Path::join` with an absolute path replaces the root outright.
    let dir = cgroup_root.join(path.trim_start_matches('/'));
    let read = |f: &str| -> Option<u64> {
        fs::read_to_string(dir.join(f)).ok().and_then(|c| cgroup_limit_kib(&c))
    };
    match (read("memory.high"), read("memory.max")) {
        (Some(h), Some(m))  => Some(h.min(m)),
        (Some(h), None)     => Some(h),
        (None, Some(m))     => Some(m),
        (None, None)        => None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    /// A fresh, empty directory under the system temp root.
    fn scratch(tag: &str) -> Outcome<PathBuf> {
        let nanos = match std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
        {
            Ok(d)  => d.as_nanos(),
            Err(_) => 0,
        };
        let dir = std::env::temp_dir().join(fmt!(
            "fe2o3_sys_resident_{}_{}_{}", tag, std::process::id(), nanos));
        res!(fs::create_dir_all(&dir));
        Ok(dir)
    }

    fn put(root: &Path, rel: &str, body: &str) -> Outcome<()> {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            res!(fs::create_dir_all(parent));
        }
        res!(fs::write(&p, body));
        Ok(())
    }

    fn status(name: &str, rss_kb: Option<u64>) -> String {
        match rss_kb {
            Some(k) => fmt!("Name:\t{}\nUmask:\t0022\nState:\tS (sleeping)\n\
                VmPeak:\t  900000 kB\nVmRSS:\t  {} kB\nThreads:\t4\n", name, k),
            None    => fmt!("Name:\t{}\nState:\tI (idle)\nThreads:\t1\n", name),
        }
    }

    #[test]
    fn status_yields_the_name_and_the_resident_set() {
        let s = status("steel", Some(412_000));
        assert_eq!(status_name_rss(&s), (Some("steel"), Some(412_000)));
        let k = status("kworker/0:1", None);
        assert_eq!(status_name_rss(&k), (Some("kworker/0:1"), None),
            "a kernel thread has a name and no resident set");
    }

    #[test]
    fn the_unified_hierarchy_line_is_the_one_read() {
        let hybrid = "12:memory:/system.slice/x.service\n0::/system.slice/steel.service\n";
        assert_eq!(cgroup_v2_path(hybrid), Some("/system.slice/steel.service"));
        assert_eq!(cgroup_v2_path("4:cpu:/\n"), None);
        assert_eq!(cgroup_limit_kib("max\n"), None, "max is no limit, not a number");
        assert_eq!(cgroup_limit_kib("1073741824\n"), Some(1_048_576));
    }

    /// A configured name longer than the kernel keeps is matched on the part the
    /// kernel keeps, and on nothing shorter.
    #[test]
    fn a_long_name_is_matched_on_the_fifteen_bytes_the_kernel_keeps() {
        assert!(comm_matches("daimond_gateway", "daimond_gateway"));
        assert!(comm_matches("a_rather_long_s", "a_rather_long_service"));
        assert!(!comm_matches("a_rather_long", "a_rather_long_service"));
        assert!(!comm_matches("steel", "steel2"));
    }

    /// Two processes of one service in one cgroup are summed and judged against
    /// that cgroup's tighter limit; a name nothing matches is reported as not
    /// running rather than dropped.
    #[test]
    fn a_walk_sums_the_processes_and_reads_their_shared_cap() -> Outcome<()> {
        let root = res!(scratch("walk"));
        let procs = root.join("proc");
        let cgroups = root.join("cgroup");
        res!(put(&procs, "101/status", &status("steel", Some(300_000))));
        res!(put(&procs, "101/cgroup", "0::/system.slice/steel.service\n"));
        res!(put(&procs, "102/status", &status("steel", Some(100_000))));
        res!(put(&procs, "102/cgroup", "0::/system.slice/steel.service\n"));
        res!(put(&procs, "200/status", &status("bash", Some(5_000))));
        res!(put(&procs, "200/cgroup", "0::/user.slice\n"));
        res!(put(&procs, "3/status", &status("steel", None)));
        res!(put(&procs, "self/status", &status("steel", Some(1))));
        // 800 MiB max, 600 MiB high: the tighter one is the cap.
        res!(put(&cgroups, "system.slice/steel.service/memory.max", "838860800\n"));
        res!(put(&cgroups, "system.slice/steel.service/memory.high", "629145600\n"));

        let names = vec![fmt!("steel"), fmt!("absent")];
        let got = res!(Resident::sample_under(&names, &procs, &cgroups));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "steel");
        assert_eq!(got[0].procs, 2,
            "two user processes match; the kernel thread and the non-numeric entry do not");
        assert_eq!(got[0].rss_kib, 400_000);
        assert_eq!(got[0].cap_kib, Some(614_400), "memory.high is the tighter limit");
        assert_eq!(got[0].cap_pct(), Some(65));
        assert_eq!(got[1], Resident { name: fmt!("absent"), ..Resident::default() },
            "a service that is not running is a reading of nothing, not an error");

        let _ = fs::remove_dir_all(&root);
        Ok(())
    }

    /// One cap over processes in different cgroups would describe neither.
    #[test]
    fn processes_in_different_cgroups_report_no_cap() -> Outcome<()> {
        let root = res!(scratch("split"));
        let procs = root.join("proc");
        let cgroups = root.join("cgroup");
        res!(put(&procs, "11/status", &status("worker", Some(1_000))));
        res!(put(&procs, "11/cgroup", "0::/system.slice/a.service\n"));
        res!(put(&procs, "12/status", &status("worker", Some(2_000))));
        res!(put(&procs, "12/cgroup", "0::/system.slice/b.service\n"));
        res!(put(&cgroups, "system.slice/a.service/memory.max", "1048576\n"));
        res!(put(&cgroups, "system.slice/b.service/memory.max", "1048576\n"));

        let got = res!(Resident::sample_under(&[fmt!("worker")], &procs, &cgroups));
        assert_eq!(got[0].procs, 2);
        assert_eq!(got[0].rss_kib, 3_000);
        assert_eq!(got[0].cap_kib, None);
        assert_eq!(got[0].cap_pct(), None);

        let _ = fs::remove_dir_all(&root);
        Ok(())
    }
}
