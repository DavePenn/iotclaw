use std::fs;
use std::path::Path;
use std::process;

const PID_FILE: &str = "data/iotclaw.pid";

/// Check if a daemon process is already running
pub fn is_running() -> bool {
    if let Ok(pid_str) = fs::read_to_string(PID_FILE) {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            // Check if process exists by sending signal 0
            #[cfg(unix)]
            {
                // kill -0 checks process existence without actually sending a signal
                let status = process::Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .status();
                return status.map_or(false, |s| s.success());
            }
            #[cfg(not(unix))]
            {
                let _ = pid;
                return false;
            }
        }
    }
    false
}

/// Write the current PID to the PID file
fn write_pid() {
    let _ = fs::create_dir_all("data");
    let pid = process::id();
    let _ = fs::write(PID_FILE, pid.to_string());
}

/// Remove the PID file
fn remove_pid() {
    let _ = fs::remove_file(PID_FILE);
}

/// Stop the running daemon by sending SIGTERM
pub fn stop_daemon() -> Result<(), String> {
    if !Path::new(PID_FILE).exists() {
        return Err("No PID file found — daemon is not running".into());
    }

    let pid_str = fs::read_to_string(PID_FILE)
        .map_err(|e| format!("Failed to read PID file: {}", e))?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|e| format!("Invalid PID in file: {}", e))?;

    #[cfg(unix)]
    {
        // Send SIGTERM
        let status = process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .map_err(|e| format!("Failed to send SIGTERM: {}", e))?;

        if status.success() {
            remove_pid();
            println!("Daemon (PID {}) stopped", pid);
            Ok(())
        } else {
            Err(format!("Failed to stop daemon PID {}", pid))
        }
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
        Err("Daemon mode is only supported on Unix systems".into())
    }
}

/// Fork the current process to run as a daemon (Unix only)
/// Returns Ok(true) if we are the daemon child, Ok(false) if we are the parent that should exit
pub fn daemonize() -> Result<bool, String> {
    if is_running() {
        return Err(format!(
            "Daemon already running (PID file: {})",
            PID_FILE
        ));
    }

    #[cfg(unix)]
    {
        // Use fork via libc
        // Safety: fork() is a standard POSIX call
        let pid = unsafe { libc_fork() };

        if pid < 0 {
            return Err("fork() failed".into());
        }

        if pid > 0 {
            // Parent process — print child PID and exit
            println!("Daemon started with PID {}", pid);
            return Ok(false); // parent should exit
        }

        // Child process
        // Create new session (detach from terminal)
        unsafe {
            libc_setsid();
        }

        // Write PID file
        write_pid();

        // Redirect stdout/stderr to log file
        // (keeping them for now since the Logger handles structured logging)

        // Install SIGTERM handler to clean up PID file
        // (simplified: the PID file will be stale if process crashes)

        Ok(true) // child continues
    }

    #[cfg(not(unix))]
    {
        Err("Daemon mode is only supported on Unix systems".into())
    }
}

// Minimal libc bindings to avoid adding libc as a dependency
#[cfg(unix)]
unsafe fn libc_fork() -> i32 {
    extern "C" {
        fn fork() -> i32;
    }
    unsafe { fork() }
}

#[cfg(unix)]
unsafe fn libc_setsid() -> i32 {
    extern "C" {
        fn setsid() -> i32;
    }
    unsafe { setsid() }
}
