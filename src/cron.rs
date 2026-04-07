use std::sync::Arc;
use tokio::sync::Mutex;

/// 定时任务
#[derive(Clone)]
pub struct CronJob {
    pub name: String,
    pub interval_secs: u64,
    pub command: String,
    pub last_run: std::time::Instant,
}

/// 定时任务管理器
pub struct CronManager {
    jobs: Arc<Mutex<Vec<CronJob>>>,
}

impl CronManager {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 添加定时任务
    pub async fn add(&self, name: &str, interval_secs: u64, command: &str) -> Result<(), String> {
        let mut jobs = self.jobs.lock().await;
        if jobs.iter().any(|j| j.name == name) {
            return Err(format!("任务 '{}' 已存在", name));
        }
        jobs.push(CronJob {
            name: name.to_string(),
            interval_secs,
            command: command.to_string(),
            last_run: std::time::Instant::now(),
        });
        Ok(())
    }

    /// 列出所有任务
    pub async fn list(&self) -> Vec<CronJob> {
        self.jobs.lock().await.clone()
    }

    /// 删除任务
    pub async fn remove(&self, name: &str) -> Result<(), String> {
        let mut jobs = self.jobs.lock().await;
        let before = jobs.len();
        jobs.retain(|j| j.name != name);
        if jobs.len() == before {
            Err(format!("任务 '{}' 不存在", name))
        } else {
            Ok(())
        }
    }

    /// 获取 jobs 的 Arc 引用（给定时循环用）
    pub fn jobs_ref(&self) -> Arc<Mutex<Vec<CronJob>>> {
        self.jobs.clone()
    }
}

/// 启动定时循环（在后台 tokio::spawn）
/// 返回 JoinHandle
pub fn start_cron_loop(
    jobs: Arc<Mutex<Vec<CronJob>>>,
    agent: Arc<Mutex<crate::agent::loop_engine::AgentLoop>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            let now = std::time::Instant::now();
            let mut due_commands: Vec<(usize, String)> = Vec::new();

            {
                let jobs_guard = jobs.lock().await;
                for (i, job) in jobs_guard.iter().enumerate() {
                    if now.duration_since(job.last_run).as_secs() >= job.interval_secs {
                        due_commands.push((i, job.command.clone()));
                    }
                }
            }

            for (idx, command) in &due_commands {
                // 更新 last_run
                {
                    let mut jobs_guard = jobs.lock().await;
                    if let Some(job) = jobs_guard.get_mut(*idx) {
                        job.last_run = now;
                    }
                }

                println!("  [Cron] 执行: {}", command);
                let mut agent_guard = agent.lock().await;
                match agent_guard.chat(command).await {
                    Ok(reply) => {
                        println!("  [Cron] 结果: {}", &reply[..reply.len().min(200)]);
                    }
                    Err(e) => {
                        eprintln!("  [Cron] 错误: {}", e);
                    }
                }
            }
        }
    })
}
