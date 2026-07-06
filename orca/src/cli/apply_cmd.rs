//! `orca apply`: confirmation flow and journal recovery around
//! `Workspace::apply`.

use orca::{EnvStore, Workspace, is_setuid_source};

use super::{confirm, format_change, select_env};

/// `orca apply [--no-upper] [--dry-run] [--yes]`.
pub fn apply(
    store: &EnvStore,
    cli_env: &Option<String>,
    no_upper: bool,
    dry_run: bool,
    yes: bool,
) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    if !nix::unistd::geteuid().is_root() {
        anyhow::bail!("orca apply requires root (try sudo)");
    }
    let ws = Workspace::open(env)?;

    // Unfinished journal from a crash/interrupt: offer recovery first.
    if ws.apply_pending() {
        return recover(&ws);
    }

    // Compute and show what would change.
    let changes = ws.apply(no_upper, false, true)?;
    if changes.is_empty() {
        println!("nothing to apply");
        return Ok(());
    }
    let mut setuid = 0usize;
    for change in &changes {
        let mark = if is_setuid_source(change) {
            setuid += 1;
            "  [setuid/setgid]"
        } else {
            ""
        };
        println!("{}{mark}", format_change(change));
    }
    println!("{} change(s) to apply to the host", changes.len());
    if setuid > 0 {
        println!("warning: {setuid} change(s) install setuid/setgid files");
    }
    if dry_run {
        return Ok(());
    }
    if !yes && !confirm("apply these changes to the host?")? {
        println!("aborted");
        return Ok(());
    }
    ws.apply(no_upper, true, false)?;
    println!("applied {} change(s)", changes.len());
    Ok(())
}

/// Interactive recovery for a leftover apply journal.
fn recover(ws: &Workspace<'_>) -> anyhow::Result<()> {
    use std::io::Write;
    let pending = ws.apply_pending_manifest()?;
    println!(
        "an unfinished apply journal with {} change(s) was found (crash or interrupt)",
        pending.len()
    );
    print!("[r]ollback the partial apply, [c]ontinue it, or [a]bort? ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    match line.trim().to_ascii_lowercase().as_str() {
        "r" | "rollback" => {
            ws.apply_rollback()?;
            println!("rolled back; re-run `orca apply` to start over");
        }
        "c" | "continue" => {
            ws.apply_resume()?;
            println!("apply completed");
        }
        _ => println!("aborted; the journal is kept"),
    }
    Ok(())
}
