//! `orca apply`: confirmation flow and journal recovery around
//! `Image::plan_apply` / `ApplyPlan::execute`.

use orca::{ApplyRecovery, EnvStore, is_setuid_source};

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
    let image = env.image()?;

    // Unfinished journal from a crash/interrupt: offer recovery first.
    if let Some(recovery) = image.pending_apply()? {
        return recover(recovery);
    }

    // Freeze the plan and show it; what is confirmed is what runs.
    let plan = image.plan_apply(no_upper)?;
    if plan.changes().is_empty() {
        println!("nothing to apply");
        return Ok(());
    }
    let mut setuid = 0usize;
    for change in plan.changes() {
        let mark = if is_setuid_source(change) {
            setuid += 1;
            "  [setuid/setgid]"
        } else {
            ""
        };
        println!("{}{mark}", format_change(change));
    }
    println!("{} change(s) to apply to the host", plan.changes().len());
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
    let count = plan.changes().len();
    plan.execute()?;
    println!("applied {count} change(s)");
    Ok(())
}

/// Interactive recovery for a leftover apply journal.
fn recover(recovery: ApplyRecovery<'_>) -> anyhow::Result<()> {
    use std::io::Write;
    println!(
        "an unfinished apply journal with {} change(s) was found (crash or interrupt)",
        recovery.changes().len()
    );
    print!("[r]ollback the partial apply, [c]ontinue it, or [a]bort? ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    match line.trim().to_ascii_lowercase().as_str() {
        "r" | "rollback" => {
            recovery.rollback()?;
            println!("rolled back; re-run `orca apply` to start over");
        }
        "c" | "continue" => {
            recovery.resume()?;
            println!("apply completed");
        }
        _ => println!("aborted; the journal is kept"),
    }
    Ok(())
}
