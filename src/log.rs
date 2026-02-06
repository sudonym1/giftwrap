use std::time::Instant;

pub struct Logger {
    verbose: bool,
}

impl Logger {
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }

    pub fn verbose(&self) -> bool {
        self.verbose
    }

    pub fn event(&self, message: impl AsRef<str>) {
        if self.verbose {
            eprintln!("[giftwrap] {}", message.as_ref());
        }
    }

    pub fn command(&self, program: &str, args: &[String]) {
        if !self.verbose {
            return;
        }

        let rendered_args = args
            .iter()
            .map(|arg| shell_escape(arg))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("[giftwrap] cmd: {program} {rendered_args}");
    }

    pub fn phase<'a>(&'a self, phase_name: &'a str) -> PhaseTimer<'a> {
        PhaseTimer {
            logger: self,
            phase_name,
            start: Instant::now(),
        }
    }
}

pub struct PhaseTimer<'a> {
    logger: &'a Logger,
    phase_name: &'a str,
    start: Instant,
}

impl Drop for PhaseTimer<'_> {
    fn drop(&mut self) {
        if self.logger.verbose {
            let elapsed_ms = self.start.elapsed().as_millis();
            eprintln!("[giftwrap] phase {}: {}ms", self.phase_name, elapsed_ms);
        }
    }
}

fn shell_escape(arg: &str) -> String {
    if arg
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_=./:".contains(ch))
    {
        return arg.to_string();
    }

    let mut escaped = String::with_capacity(arg.len() + 2);
    escaped.push('\'');
    for ch in arg.chars() {
        if ch == '\'' {
            escaped.push_str("'\\''");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}
