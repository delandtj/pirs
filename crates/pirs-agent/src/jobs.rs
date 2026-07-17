use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq)]
pub enum JobStatus {
    Running,
    Exited(i32),
    Killed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Bash,
    Agent,
}

pub struct Job {
    pub id: u64,
    pub kind: JobKind,
    pub description: String,
    pub status: JobStatus,
    pub output_path: PathBuf,
    pub started_at: u64,
    pub pid: Option<u32>,
    pub progress: Option<Arc<Mutex<String>>>,
    pub steer: Option<NotifyFn>,
}

impl Job {
    pub fn status_line(&self) -> String {
        let status = match &self.status {
            JobStatus::Running => "running".to_string(),
            JobStatus::Exited(code) => format!("exited({code})"),
            JobStatus::Killed => "killed".to_string(),
        };
        format!("#{} [{}] {} — {}", self.id, status, kind_name(self.kind), self.description)
    }
}

fn kind_name(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Bash => "bash",
        JobKind::Agent => "agent",
    }
}

pub type NotifyFn = Arc<dyn Fn(String) + Send + Sync>;

pub struct JobRegistry {
    jobs: Mutex<HashMap<u64, Arc<Mutex<Job>>>>,
    next_id: Mutex<u64>,
    notifier: Mutex<Option<NotifyFn>>,
}

static REGISTRY: OnceLock<JobRegistry> = OnceLock::new();

pub fn registry() -> &'static JobRegistry {
    REGISTRY.get_or_init(|| JobRegistry {
        jobs: Mutex::new(HashMap::new()),
        next_id: Mutex::new(1),
        notifier: Mutex::new(None),
    })
}

impl JobRegistry {
    pub fn set_notifier(&self, notify: NotifyFn) {
        *self.notifier.lock().unwrap() = Some(notify);
    }

    pub fn notify(&self, message: impl Into<String>) {
        let cb = self.notifier.lock().unwrap().clone();
        if let Some(cb) = cb {
            cb(message.into());
        }
    }

    pub fn register(
        &self,
        kind: JobKind,
        description: String,
        output_path: PathBuf,
        pid: Option<u32>,
    ) -> (u64, Arc<Mutex<Job>>) {
        let id = {
            let mut n = self.next_id.lock().unwrap();
            let id = *n;
            *n += 1;
            id
        };
        let job = Arc::new(Mutex::new(Job {
            id,
            kind,
            description,
            status: JobStatus::Running,
            output_path,
            started_at: pirs_ai::now_millis(),
            pid,
            progress: None,
            steer: None,
        }));
        self.jobs.lock().unwrap().insert(id, Arc::clone(&job));
        (id, job)
    }

    pub fn set_status(&self, id: u64, status: JobStatus) {
        if let Some(job) = self.jobs.lock().unwrap().get(&id) {
            job.lock().unwrap().status = status.clone();
        }
    }

    pub fn set_progress_handle(&self, id: u64, progress: Arc<Mutex<String>>) {
        if let Some(job) = self.jobs.lock().unwrap().get(&id) {
            job.lock().unwrap().progress = Some(progress);
        }
    }

    pub fn set_steer(&self, id: u64, steer: NotifyFn) {
        if let Some(job) = self.jobs.lock().unwrap().get(&id) {
            job.lock().unwrap().steer = Some(steer);
        }
    }

    pub fn steer(&self, id: u64, message: &str) -> Result<(), String> {
        let jobs = self.jobs.lock().unwrap();
        let Some(job) = jobs.get(&id) else {
            return Err(format!("no such job: {id}"));
        };
        let job = job.lock().unwrap();
        let Some(steer) = &job.steer else {
            return Err(format!("job {id} is not steerable"));
        };
        steer(message.to_string());
        Ok(())
    }

    pub fn get(&self, id: u64) -> Option<Arc<Mutex<Job>>> {
        self.jobs.lock().unwrap().get(&id).cloned()
    }

    pub fn list(&self) -> Vec<String> {
        self.jobs
            .lock()
            .unwrap()
            .values()
            .map(|j| j.lock().unwrap().status_line())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_list_steer() {
        let r = registry();
        let (id, _job) = r.register(JobKind::Bash, "sleep 1".into(), PathBuf::from("/tmp/x.log"), Some(1));
        assert!(r.list().iter().any(|l| l.contains(&format!("#{id}"))));
        assert!(r.steer(id, "hi").is_err(), "bash jobs are not steerable");
        let got = Arc::new(Mutex::new(String::new()));
        let got2 = Arc::clone(&got);
        r.set_steer(id, Arc::new(move |m| {
            *got2.lock().unwrap() = m;
        }));
        r.steer(id, "hello job").unwrap();
        assert_eq!(*got.lock().unwrap(), "hello job");
        r.set_status(id, JobStatus::Exited(0));
        assert!(r.list().iter().any(|l| l.contains("exited(0)")));
    }

    #[test]
    fn notifier_fires() {
        let r = registry();
        let got = Arc::new(Mutex::new(String::new()));
        let got2 = Arc::clone(&got);
        r.set_notifier(Arc::new(move |m| {
            *got2.lock().unwrap() = m;
        }));
        r.notify("job done");
        assert_eq!(*got.lock().unwrap(), "job done");
    }
}
