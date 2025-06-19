pub struct WrappedScreen(pub crate::Screen);

impl vte::Perform for WrappedScreen {
    fn print(&mut self, c: char) {
        if c == '\u{fffd}' || ('\u{80}'..'\u{a0}').contains(&c) {
            log::debug!("unhandled text character: {c}");
        }
        self.0.text(c);
    }

    fn execute(&mut self, b: u8) {
        match b {
            8 => self.0.bs(),
            9 => self.0.tab(),
            10 => self.0.lf(),
            11 => self.0.vt(),
            12 => self.0.ff(),
            13 => self.0.cr(),
            // we don't implement shift in/out alternate character sets, but
            // it shouldn't count as an "error"
            7 | 14 | 15 => {}
            _ => {
                log::debug!("unhandled control character: {b}");
            }
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, b: u8) {
        intermediates.first().map_or_else(
            || match b {
                b'7' => self.0.decsc(),
                b'8' => self.0.decrc(),
                b'=' => self.0.deckpam(),
                b'>' => self.0.deckpnm(),
                b'M' => self.0.ri(),
                b'c' => self.0.ris(),
                b'g' => {}
                _ => {
                    log::debug!("unhandled escape code: ESC {b}");
                }
            },
            |i| {
                log::debug!("unhandled escape code: ESC {i} {b}");
            },
        );
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        c: char,
    ) {
        match intermediates.first() {
            None => match c {
                '@' => self.0.ich(canonicalize_params_1(params, 1)),
                'A' => self.0.cuu(canonicalize_params_1(params, 1)),
                'B' => self.0.cud(canonicalize_params_1(params, 1)),
                'C' => self.0.cuf(canonicalize_params_1(params, 1)),
                'D' => self.0.cub(canonicalize_params_1(params, 1)),
                'E' => self.0.cnl(canonicalize_params_1(params, 1)),
                'F' => self.0.cpl(canonicalize_params_1(params, 1)),
                'G' => self.0.cha(canonicalize_params_1(params, 1)),
                'H' => self.0.cup(canonicalize_params_2(params, 1, 1)),
                'J' => self.0.ed(canonicalize_params_1(params, 0)),
                'K' => self.0.el(canonicalize_params_1(params, 0)),
                'L' => self.0.il(canonicalize_params_1(params, 1)),
                'M' => self.0.dl(canonicalize_params_1(params, 1)),
                'P' => self.0.dch(canonicalize_params_1(params, 1)),
                'S' => self.0.su(canonicalize_params_1(params, 1)),
                'T' => self.0.sd(canonicalize_params_1(params, 1)),
                'X' => self.0.ech(canonicalize_params_1(params, 1)),
                'd' => self.0.vpa(canonicalize_params_1(params, 1)),
                'h' => self.0.sm(params),
                'l' => self.0.rm(params),
                'm' => self.0.sgr(params),
                'r' => self.0.decstbm(canonicalize_params_decstbm(
                    params,
                    self.0.grid().size(),
                )),
                't' => self.0.xtwinops(params),
                _ => {
                    if log::log_enabled!(log::Level::Debug) {
                        log::debug!(
                            "unhandled csi sequence: CSI {} {}",
                            param_str(params),
                            c
                        );
                    }
                }
            },
            Some(b'?') => match c {
                'J' => self.0.decsed(canonicalize_params_1(params, 0)),
                'K' => self.0.decsel(canonicalize_params_1(params, 0)),
                'h' => self.0.decset(params),
                'l' => self.0.decrst(params),
                _ => {
                    if log::log_enabled!(log::Level::Debug) {
                        log::debug!(
                            "unhandled csi sequence: CSI ? {} {}",
                            param_str(params),
                            c
                        );
                    }
                }
            },
            Some(i) => {
                if log::log_enabled!(log::Level::Debug) {
                    log::debug!(
                        "unhandled csi sequence: CSI {} {} {}",
                        i,
                        param_str(params),
                        c
                    );
                }
            }
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bel_terminated: bool) {
        match (params.first(), params.get(1)) {
            (Some(&b"0"), Some(s)) => self.0.osc0(s),
            (Some(&b"1"), Some(s)) => self.0.osc1(s),
            (Some(&b"2"), Some(s)) => self.0.osc2(s),
            _ => {
                if log::log_enabled!(log::Level::Debug) {
                    log::debug!(
                        "unhandled osc sequence: OSC {}",
                        osc_param_str(params),
                    );
                }
            }
        }
    }

    fn hook(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        if log::log_enabled!(log::Level::Debug) {
            intermediates.first().map_or_else(
                || {
                    log::debug!(
                        "unhandled dcs sequence: DCS {} {}",
                        param_str(params),
                        action,
                    );
                },
                |i| {
                    log::debug!(
                        "unhandled dcs sequence: DCS {} {} {}",
                        i,
                        param_str(params),
                        action,
                    );
                },
            );
        }
    }
}

fn canonicalize_params_1(params: &vte::Params, default: u16) -> u16 {
    let first = params.iter().next().map_or(0, |x| *x.first().unwrap_or(&0));
    if first == 0 { default } else { first }
}

fn canonicalize_params_2(
    params: &vte::Params,
    default1: u16,
    default2: u16,
) -> (u16, u16) {
    let mut iter = params.iter();
    let first = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let first = if first == 0 { default1 } else { first };

    let second = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let second = if second == 0 { default2 } else { second };

    (first, second)
}

fn canonicalize_params_decstbm(
    params: &vte::Params,
    size: crate::grid::Size,
) -> (u16, u16) {
    let mut iter = params.iter();
    let top = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let top = if top == 0 { 1 } else { top };

    let bottom = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let bottom = if bottom == 0 { size.rows } else { bottom };

    (top, bottom)
}

pub fn param_str(params: &vte::Params) -> String {
    let strs: Vec<_> = params
        .iter()
        .map(|subparams| {
            let subparam_strs: Vec<_> = subparams
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            subparam_strs.join(" : ")
        })
        .collect();
    strs.join(" ; ")
}

fn osc_param_str(params: &[&[u8]]) -> String {
    let strs: Vec<_> = params
        .iter()
        .map(|b| format!("\"{}\"", std::string::String::from_utf8_lossy(b)))
        .collect();
    strs.join(" ; ")
}

pub struct WrappedScreenWithCallbacks<'a, T: crate::callbacks::Callbacks> {
    screen: &'a mut crate::perform::WrappedScreen,
    callbacks: &'a mut T,
}

impl<'a, T: crate::callbacks::Callbacks> WrappedScreenWithCallbacks<'a, T> {
    pub fn new(
        screen: &'a mut crate::perform::WrappedScreen,
        callbacks: &'a mut T,
    ) -> Self {
        Self { screen, callbacks }
    }
}

impl<T: crate::callbacks::Callbacks> vte::Perform
    for WrappedScreenWithCallbacks<'_, T>
{
    fn print(&mut self, c: char) {
        if c == '\u{fffd}' || ('\u{80}'..'\u{a0}').contains(&c) {
            self.callbacks.error(&mut self.screen.0);
        }
        self.screen.print(c);
    }

    fn execute(&mut self, b: u8) {
        match b {
            7 => self.callbacks.audible_bell(&mut self.screen.0),
            8..=15 => {}
            _ => {
                self.callbacks.error(&mut self.screen.0);
            }
        }
        self.screen.execute(b);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, b: u8) {
        if intermediates.is_empty() && b == b'g' {
            self.callbacks.visual_bell(&mut self.screen.0);
        }
        self.screen.esc_dispatch(intermediates, ignore, b);
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        c: char,
    ) {
        if intermediates.is_empty() && c == 't' {
            let mut iter = params.iter();
            let op = iter.next().and_then(|x| x.first().copied());
            if op == Some(8) {
                let (screen_rows, screen_cols) = self.screen.0.size();
                let rows = iter.next().map_or(screen_rows, |x| {
                    *x.first().unwrap_or(&screen_rows)
                });
                let cols = iter.next().map_or(screen_cols, |x| {
                    *x.first().unwrap_or(&screen_cols)
                });
                self.callbacks.resize(&mut self.screen.0, (rows, cols));
            }
        }
        self.screen.csi_dispatch(params, intermediates, ignore, c);
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bel_terminated: bool) {
        self.screen.osc_dispatch(params, bel_terminated);
    }

    fn hook(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        self.screen.hook(params, intermediates, ignore, action);
    }
}
