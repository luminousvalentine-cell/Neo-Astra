pub trait Host {

    fn stdout(&mut self, text: &str);

    fn stderr(&mut self, text: &str);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StdHost;

impl Host for StdHost {
    fn stdout(&mut self, text: &str) {
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    }

    fn stderr(&mut self, text: &str) {
        use std::io::Write;
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(text.as_bytes());
        let _ = stderr.flush();
    }
}

#[derive(Debug, Default, Clone)]
pub struct BufferHost {
    pub stdout: String,
    pub stderr: String,
}

impl BufferHost {

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Host for BufferHost {
    fn stdout(&mut self, text: &str) {
        self.stdout.push_str(text);
    }

    fn stderr(&mut self, text: &str) {
        self.stderr.push_str(text);
    }
}
