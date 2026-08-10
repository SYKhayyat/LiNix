//! `linix repl` — the resolved image, with a cursor (U34, XIII.31).
//!
//! Every question this answers is one `linix eval | jq` can answer too; the repl is the same
//! answers reached by trying, not by reading the manual. Its non-negotiable (the U20 rule) is
//! that it is a thin front end over the **one** parser and resolver the binary already uses —
//! never a second implementation. So every line below delegates:
//!
//! - a bare or prefixed name → `StateResolver::resolve_spec`, the same resolution `sync` does,
//!   so "what does `ripgrep` become here" is answered by the machine, not by a guess;
//! - `when <expr>` → `config::parser::eval_when` against this host's facts, the same predicate
//!   evaluator every `when` block goes through;
//! - `:vars` / `:eval` → the resolver's own `resolve_vars` / `resolve_model`.
//!
//! It is read-only and takes no locks (`Commands::writes` answers false for it): it resolves and prints,
//! it never touches the machine.

use crate::app::sync::resolver::StateResolver;
use crate::app::App;
use crate::core::Result;
use std::io::{BufRead, Write};

const HELP: &str = "\
linix repl — resolve names, evaluate `when`, and inspect the model against THIS machine.

  <name>            resolve a bare or prefixed name, e.g. `ripgrep` or `cargo:ripgrep`
  when <expr>       evaluate a `when` predicate here, e.g. `when os == linux`
  :vars             the variables every `when` is decided against
  :eval             the full resolved desired state, as JSON
  :help             this text
  :quit  /  Ctrl-D  leave";

/// Run the interactive loop. Blocking stdin/stdout is deliberate — a prompt is a person at a
/// keyboard, not a pipeline (that is what `linix eval` is for).
pub async fn run(app: &App) -> Result<()> {
    let resolver = StateResolver::new(&app.config, app.registry.clone(), false).await;

    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    println!("linix repl — `:help` for commands, `:quit` to leave.");
    loop {
        print!("linix> ");
        std::io::stdout().flush().ok();
        let Some(line) = lines.next() else {
            println!();
            break; // Ctrl-D / EOF
        };
        let line = line.map_err(|e| crate::core::Error::Io(e.to_string()))?;
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        match evaluate(app, &resolver, input).await {
            Command::Continue => {}
            Command::Quit => break,
        }
    }
    Ok(())
}

enum Command {
    Continue,
    Quit,
}

async fn evaluate(app: &App, resolver: &StateResolver<'_>, input: &str) -> Command {
    match input {
        ":quit" | ":q" | "quit" | "exit" => return Command::Quit,
        ":help" | "help" | "?" => {
            println!("{}", HELP);
            return Command::Continue;
        }
        ":vars" => {
            print_vars(resolver).await;
            return Command::Continue;
        }
        ":eval" => {
            print_eval(app, resolver).await;
            return Command::Continue;
        }
        _ => {}
    }

    if let Some(pred) = crate::config::grammar::when_predicate(input) {
        eval_when(resolver, pred).await;
        return Command::Continue;
    }

    resolve_name(resolver, input).await;
    Command::Continue
}

async fn print_vars(resolver: &StateResolver<'_>) {
    match resolver.resolve_vars().await {
        Ok(vars) if vars.is_empty() => println!("no variables"),
        Ok(vars) => {
            let width = vars.keys().map(|k| k.len()).max().unwrap_or(0);
            for (k, v) in &vars {
                println!("  {:width$}  {}", k, v, width = width);
            }
        }
        Err(e) => println!("error: {}", e),
    }
}

async fn print_eval(app: &App, resolver: &StateResolver<'_>) {
    match resolver.resolve_model().await {
        Ok(state) => {
            let doc = crate::app::eval::Evaluation::of(&state, &app.config.config_root());
            match doc.render() {
                Ok(json) => print!("{}", json),
                Err(e) => println!("error: {}", e),
            }
        }
        Err(e) => println!("error: {}", e),
    }
}

async fn eval_when(resolver: &StateResolver<'_>, pred: &str) {
    let facts = match resolver.facts_for_host().await {
        Ok(f) => f,
        Err(e) => {
            println!("error: {}", e);
            return;
        }
    };
    match crate::config::parser::eval_when(pred, &facts) {
        Ok(true) => println!("true"),
        Ok(false) => println!("false"),
        Err(e) => println!("error: {}", e),
    }
}

async fn resolve_name(resolver: &StateResolver<'_>, spec: &str) {
    match resolver.resolve_spec(spec).await {
        Ok(specs) if specs.is_empty() => {
            println!("`{}` resolves to nothing on this machine", spec)
        }
        Ok(specs) => {
            for s in specs {
                match s.options.one("version") {
                    Some(v) => println!("  {}:{}@{}", s.backend, s.name, v),
                    None => println!("  {}:{}", s.backend, s.name),
                }
            }
        }
        Err(e) => println!("cannot resolve `{}`: {}", spec, e),
    }
}
