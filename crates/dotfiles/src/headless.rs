use dotfiles_core::paths::Paths;
use dotfiles_core::pipeline::{run_pipeline, PipelineOptions};
use dotfiles_core::steps::PipelineEvent;
use std::sync::mpsc;

#[allow(dead_code)]
pub fn run_headless(paths: &Paths, trigger: &str) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<PipelineEvent>();
    // Spawn a thread to print JSON-lines for each event
    let handle = std::thread::spawn(move || {
        for ev in rx {
            let line = match ev {
                PipelineEvent::StepStarted { name, index, total } => {
                    serde_json::json!({"type":"step_started","name":name,"index":index,"total":total}).to_string()
                }
                PipelineEvent::LogLine { step, line, .. } => {
                    serde_json::json!({"type":"log_line","step":step,"line":line}).to_string()
                }
                PipelineEvent::StepFinished { report } => {
                    serde_json::json!({"type":"step_finished","report":report}).to_string()
                }
                PipelineEvent::RunFinished { status, report_path } => {
                    serde_json::json!({"type":"run_finished","status":status,"report_path":report_path}).to_string()
                }
                PipelineEvent::SudoPrompt { command, reason, respond } => {
                    eprint!("Sudo required: {} ({}): ", command, reason);
                    let _ = std::io::Write::flush(&mut std::io::stderr());
                    let mut pw = String::new();
                    let _ = std::io::stdin().read_line(&mut pw);
                    let _ = respond.send(pw.trim().to_string());
                    continue;
                }
            };
            println!("{}", line);
        }
    });

    let askpass = crate::upgrade::askpass_wrapper_path();
    let opts = PipelineOptions {
        trigger: trigger.to_string(),
        sudo_askpass: askpass,
        event_tx: Some(tx),
    };
    let (_report, path) = run_pipeline(paths, opts)?;
    // drop sender so printer exits
    // need to wait a bit
    std::thread::sleep(std::time::Duration::from_millis(200));
    drop(handle);
    eprintln!("report: {}", path.display());
    Ok(())
}
