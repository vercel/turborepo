/// A collection of signals that are caught by the listeners
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Signal {
    #[cfg(windows)]
    CtrlC,
    #[cfg(not(windows))]
    Interrupt,
    #[cfg(not(windows))]
    Terminate,
}
