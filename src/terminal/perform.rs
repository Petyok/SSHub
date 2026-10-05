const BASE64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
const CLIPBOARD_SELECTOR: &[u8] = b"cpqs01234567";

pub struct WrappedScreen<CB: crate::terminal::callbacks::Callbacks<T> = (), T = ()> {
    pub screen: crate::terminal::screen::Screen<T>,
    pub callbacks: CB,
}

impl WrappedScreen<()> {
    pub fn new(rows: u16, cols: u16, scrollback_len: usize) -> Self {
        Self::new_with_callbacks(rows, cols, scrollback_len, ())
    }
}

impl<CB: crate::terminal::callbacks::Callbacks<T>, T> WrappedScreen<CB, T> {
    pub fn new_with_callbacks(rows: u16, cols: u16, scrollback_len: usize, callbacks: CB) -> Self {
        Self {
            screen: crate::terminal::screen::Screen::new(
                crate::terminal::grid::Size { rows, cols },
                scrollback_len,
            ),
            callbacks,
        }
    }
}

impl<CB: crate::terminal::callbacks::Callbacks<T>, T> vte::Perform for WrappedScreen<CB, T> {
    fn print(&mut self, c: char) {
        if c == '\u{fffd}' || ('\u{80}'..'\u{a0}').contains(&c) {
            self.callbacks.unhandled_char(&mut self.screen, c);
        } else {
            self.screen.text(c);
        }
    }

    fn execute(&mut self, b: u8) {
        match b {
            7 => self.callbacks.audible_bell(&mut self.screen),
            8 => self.screen.bs(),
            9 => self.screen.tab(),
            10 => self.screen.lf(),
            11 => self.screen.vt(),
            12 => self.screen.ff(),
            13 => self.screen.cr(),
            // we don't implement shift in/out alternate character sets, but
            // it shouldn't count as an "error"
            14 | 15 => {}
            _ => self.callbacks.unhandled_control(&mut self.screen, b),
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, b: u8) {
        if let Some(i) = intermediates.first() {
            // shellglass: SCS `ESC ( B` / `ESC ) B` designates US-ASCII into
            // G0/G1 — the only charset this crate models, so it's already
            // true; deliberately ignored. Any other designation (e.g.
            // `ESC ( 0`, DEC line drawing) is a real gap and keeps reporting.
            if matches!(*i, b'(' | b')') && b == b'B' {
                return;
            }
            self.callbacks.unhandled_escape(
                &mut self.screen,
                Some(*i),
                intermediates.get(1).copied(),
                b,
            );
        } else {
            match b {
                b'7' => self.screen.decsc(),
                b'8' => self.screen.decrc(),
                b'=' => self.screen.deckpam(),
                b'>' => self.screen.deckpnm(),
                b'H' => self.screen.hts(), // shellglass: HTS
                b'M' => self.screen.ri(),
                // shellglass: ST — vte ends an OSC/DCS string itself and then
                // reports the terminator as a bare escape; pure syntax,
                // deliberately ignored (not a fidelity gap).
                b'\\' => {}
                b'c' => self.screen.ris(),
                b'g' => self.callbacks.visual_bell(&mut self.screen),
                _ => {
                    self.callbacks
                        .unhandled_escape(&mut self.screen, None, None, b);
                }
            }
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], _ignore: bool, c: char) {
        let unhandled = |screen: &mut crate::terminal::screen::Screen<T>| {
            self.callbacks.unhandled_csi(
                screen,
                intermediates.first().copied(),
                intermediates.get(1).copied(),
                &params.iter().collect::<Vec<_>>(),
                c,
            );
        };
        match intermediates.first() {
            None => match c {
                '@' => self.screen.ich(canonicalize_params_1(params, 1)),
                'A' => self.screen.cuu(canonicalize_params_1(params, 1)),
                // shellglass: VPR (e) moves like CUD
                'B' | 'e' => {
                    self.screen.cud(canonicalize_params_1(params, 1));
                }
                // shellglass: HPR (a) moves like CUF
                'C' | 'a' => {
                    self.screen.cuf(canonicalize_params_1(params, 1));
                }
                'D' => self.screen.cub(canonicalize_params_1(params, 1)),
                'E' => self.screen.cnl(canonicalize_params_1(params, 1)),
                'F' => self.screen.cpl(canonicalize_params_1(params, 1)),
                // shellglass: HPA (`) moves like CHA
                'G' | '`' => {
                    self.screen.cha(canonicalize_params_1(params, 1));
                }
                // shellglass: HVP (f) is the CUP alias
                'H' | 'f' => {
                    self.screen.cup(canonicalize_params_2(params, 1, 1));
                }
                // shellglass: CHT
                'I' => self.screen.cht(canonicalize_params_1(params, 1)),
                'J' => self.screen.ed(canonicalize_params_1(params, 0), unhandled),
                'K' => self.screen.el(canonicalize_params_1(params, 0), unhandled),
                'L' => self.screen.il(canonicalize_params_1(params, 1)),
                'M' => self.screen.dl(canonicalize_params_1(params, 1)),
                'P' => self.screen.dch(canonicalize_params_1(params, 1)),
                'S' => self.screen.su(canonicalize_params_1(params, 1)),
                'T' => self.screen.sd(canonicalize_params_1(params, 1)),
                'X' => self.screen.ech(canonicalize_params_1(params, 1)),
                // shellglass: CBT
                'Z' => self.screen.cbt(canonicalize_params_1(params, 1)),
                // shellglass: REP
                'b' => self.screen.rep(canonicalize_params_1(params, 1)),
                'd' => self.screen.vpa(canonicalize_params_1(params, 1)),
                // shellglass: Primary DA and standard DSR — identity/status
                // queries with zero render effect; answering is the embedding
                // terminal's job. (ConPTY emits `CSI 6 n` at startup.)
                // Deliberately ignored, not unhandled.
                'c' | 'n' => {}
                // shellglass: TBC
                'g' => self.screen.tbc(canonicalize_params_1(params, 0)),
                // shellglass: SM/RM (IRM is the one modeled mode)
                'h' => self.screen.sm(params, unhandled),
                'l' => self.screen.rm(params, unhandled),
                'm' => self.screen.sgr(params, unhandled),
                'r' => self.screen.decstbm(canonicalize_params_decstbm(
                    params,
                    self.screen.grid().size(),
                )),
                // shellglass: SCOSC/SCORC, only in their bare parameterless
                // forms (vte hands a bare CSI a single default-0 param) —
                // `CSI <n> s` would be DECSLRM (unsupported, and its
                // left/right-margin semantics must not save the cursor).
                's' if canonicalize_params_1(params, 0) == 0 => {
                    self.screen.scosc();
                }
                'u' if canonicalize_params_1(params, 0) == 0 => {
                    self.screen.scorc();
                }
                't' => {
                    let mut params_iter = params.iter();
                    let op = params_iter.next().and_then(|x| x.first().copied());
                    match op {
                        Some(8) => {
                            let (screen_rows, screen_cols) = self.screen.size();
                            let rows = params_iter
                                .next()
                                .map_or(screen_rows, |x| *x.first().unwrap_or(&screen_rows));
                            let cols = params_iter
                                .next()
                                .map_or(screen_cols, |x| *x.first().unwrap_or(&screen_cols));
                            self.callbacks.resize(&mut self.screen, (rows, cols));
                        }
                        // XTWINOPS reports have no screen effect. The SSHub
                        // observer answers 18 (grid size); the other listed
                        // queries remain unanswered.
                        Some(11 | 13 | 14 | 16 | 18 | 19 | 21) => {}
                        // Title save/restore retains bounded model metadata;
                        // SSHub does not change its header or outer title.
                        // Sub-param 1 is icon name only — skip it.
                        Some(op @ (22 | 23)) => {
                            let which = params_iter
                                .next()
                                .and_then(|x| x.first().copied())
                                .unwrap_or(0);
                            if which != 1 {
                                if op == 22 {
                                    self.screen.title_push();
                                } else {
                                    self.screen.title_pop();
                                }
                            }
                        }
                        _ => {
                            self.callbacks.unhandled_csi(
                                &mut self.screen,
                                None,
                                None,
                                &params.iter().collect::<Vec<_>>(),
                                c,
                            );
                        }
                    }
                }
                _ => {
                    self.callbacks.unhandled_csi(
                        &mut self.screen,
                        None,
                        None,
                        &params.iter().collect::<Vec<_>>(),
                        c,
                    );
                }
            },
            Some(b'?') => match c {
                'J' => self
                    .screen
                    .decsed(canonicalize_params_1(params, 0), unhandled),
                'K' => self
                    .screen
                    .decsel(canonicalize_params_1(params, 0), unhandled),
                'h' => self.screen.decset(params, unhandled),
                'l' => self.screen.decrst(params, unhandled),
                // shellglass: private DSR (`CSI ? … n` — DECXCPR cursor
                // position, printer/UDK/locator status) and kitty keyboard
                // protocol query (`CSI ? u`): SSHub intentionally leaves these
                // unanswered (PR #114). DECRQM (`CSI ? Ps $ p`, mode query — tmux and
                // neovim probe 2026/2027 with it) is the same class: a
                // question, not a state change.
                'n' | 'u' => {}
                'p' if intermediates.get(1) == Some(&b'$') => {}
                _ => {
                    self.callbacks.unhandled_csi(
                        &mut self.screen,
                        Some(b'?'),
                        intermediates.get(1).copied(),
                        &params.iter().collect::<Vec<_>>(),
                        c,
                    );
                }
            },
            // shellglass: Secondary DA (`CSI > c`) — same identity-query
            // family as Primary DA above; deliberately ignored. XTVERSION
            // (`CSI > q`) likewise — SSHub does not advertise these capabilities.
            // XTMODKEYS (`CSI > 4 m` and friends) and kitty keyboard
            // push (`CSI > … u`) are input protocol with zero render effect.
            Some(b'>') => match c {
                'c' | 'q' | 'm' | 'u' => {}
                _ => {
                    self.callbacks.unhandled_csi(
                        &mut self.screen,
                        Some(b'>'),
                        intermediates.get(1).copied(),
                        &params.iter().collect::<Vec<_>>(),
                        c,
                    );
                }
            },
            // Kitty keyboard pop has no screen effect. SSHub does not
            // implement or advertise this input protocol (PR #114).
            Some(b'<') if c == 'u' => {}
            // shellglass: DECSCUSR (`CSI n SP q`, set cursor style) — 0-6
            // (default / blinking+steady block / underline / bar); anything
            // else keeps reporting.
            Some(b' ') => match (c, canonicalize_params_1(params, 0)) {
                ('q', style @ 0..=6) => {
                    // 0-6 always fits u8
                    self.screen.decscusr(u8::try_from(style).unwrap());
                }
                _ => {
                    self.callbacks.unhandled_csi(
                        &mut self.screen,
                        Some(b' '),
                        intermediates.get(1).copied(),
                        &params.iter().collect::<Vec<_>>(),
                        c,
                    );
                }
            },
            // shellglass: ANSI-mode DECRQM (`CSI Ps $ p`) — the non-private
            // half of the query above; likewise answered by the terminal.
            Some(b'$') if c == 'p' => {}
            // shellglass: intermediate `!`
            Some(b'!') => match c {
                // DECSTR, soft terminal reset
                'p' => self.screen.decstr(),
                _ => {
                    self.callbacks.unhandled_csi(
                        &mut self.screen,
                        Some(b'!'),
                        intermediates.get(1).copied(),
                        &params.iter().collect::<Vec<_>>(),
                        c,
                    );
                }
            },
            Some(i) => {
                self.callbacks.unhandled_csi(
                    &mut self.screen,
                    Some(*i),
                    intermediates.get(1).copied(),
                    &params.iter().collect::<Vec<_>>(),
                    c,
                );
            }
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bel_terminated: bool) {
        match params {
            [b"0", s] => {
                // shellglass: the title is screen state now (the mirror's
                // page follows it); the callbacks stay for embedders.
                self.screen.set_title(s);
                self.callbacks.set_window_icon_name(&mut self.screen, s);
                self.callbacks.set_window_title(&mut self.screen, s);
            }
            [b"1", s] => {
                self.callbacks.set_window_icon_name(&mut self.screen, s);
            }
            [b"2", s] => {
                self.screen.set_title(s); // shellglass (see OSC 0)
                self.callbacks.set_window_title(&mut self.screen, s);
            }
            // shellglass: default fg/bg *queries* (vim/neovim background
            // detection) are not currently answered by SSHub; OSC 133
            // shell-integration prompt/command markers are semantic metadata.
            // Neither renders directly; SSHub currently leaves these
            // unsupported queries unanswered.
            [b"10" | b"11", b"?"] | [b"133", ..] => {}
            // shellglass: OSC 4 palette *queries* (`4 ; idx ; ?`, repeatable
            // in pairs) — same class: unsupported in SSHub, nothing
            // renders. The SET form (`4 ; idx ; spec`) and OSC 104 reset do
            // change rendering, so they stay unhandled and keep reporting
            // until the palette is modelled.
            [b"4", pairs @ ..]
                if !pairs.is_empty()
                    && pairs.len() % 2 == 0
                    && pairs.chunks(2).all(|pair| pair[1] == b"?") => {}
            // shellglass: OSC 10/11 set form — override the default fg/bg
            // (theme switchers, OSC 11-emitting TUIs); OSC 110/111 reset.
            // A value we can't parse (named colors, rgbi:) stays unhandled
            // so it keeps reporting instead of silently mis-rendering.
            [b"10", value] => match parse_osc_color(value) {
                Some(c) => self.screen.set_default_fg(Some(c)),
                None => {
                    self.callbacks.unhandled_osc(&mut self.screen, params);
                }
            },
            [b"11", value] => match parse_osc_color(value) {
                Some(c) => self.screen.set_default_bg(Some(c)),
                None => {
                    self.callbacks.unhandled_osc(&mut self.screen, params);
                }
            },
            [b"110"] => self.screen.set_default_fg(None),
            [b"111"] => self.screen.set_default_bg(None),
            // shellglass: OSC 8 hyperlinks — `8 ; params ; URI`. The params
            // (kitty's `id=` continuity hint) are ignored: the mirror renders
            // each cell as an anchor, so multi-part unification buys nothing.
            // vte splits the payload on `;`, so a URI containing `;` arrives
            // as extra params — rejoin them. An empty URI closes the link.
            [b"8", _params, uri @ ..] => {
                if uri.iter().all(|part| part.is_empty()) {
                    self.screen.link_close();
                } else {
                    self.screen.link_open(&uri.join(&b';'));
                }
            }
            [b"52", ty, data] => match (ty.iter().all(|c| CLIPBOARD_SELECTOR.contains(c)), *data) {
                (true, b"?") => {
                    self.callbacks.paste_from_clipboard(&mut self.screen, ty);
                }
                (true, data) if data.iter().all(|c| BASE64.contains(c)) => {
                    self.callbacks.copy_to_clipboard(&mut self.screen, ty, data);
                }
                _ => {
                    self.callbacks.unhandled_osc(&mut self.screen, params);
                }
            },
            _ => {
                self.callbacks.unhandled_osc(&mut self.screen, params);
            }
        }
    }
}

// shellglass: the two XParseColor shapes real emitters use — `rgb:R/G/B`
// (1-4 hex digits per component, left-aligned fractions: scale to 8 bits) and
// `#R…G…B…` (3/6/9/12 digits, raw bit patterns: take the top 8 bits, with
// 4-bit components replicated like XParseColor). Named colors and `rgbi:`
// return None (→ unhandled, so telemetry keeps flagging them).
fn parse_osc_color(value: &[u8]) -> Option<(u8, u8, u8)> {
    fn hexval(digits: &[u8]) -> Option<u16> {
        if digits.is_empty() || digits.len() > 4 {
            return None;
        }
        let s = std::str::from_utf8(digits).ok()?;
        u16::from_str_radix(s, 16).ok()
    }
    // A component of `n` hex digits scaled to 8 bits: one digit replicates
    // (0xA → 0xAA), two passes through, three/four keep the top byte.
    fn scale(v: u16, digits: usize) -> u8 {
        let byte = match digits {
            1 => v * 0x11,
            2 => v,
            3 => v >> 4,
            _ => v >> 8,
        };
        // hexval bounds v by the digit count, so this is always ≤ 0xFF
        u8::try_from(byte).unwrap_or(u8::MAX)
    }
    if let Some(rgb) = value.strip_prefix(b"rgb:") {
        let mut parts = rgb.split(|&b| b == b'/');
        let (r, g, b) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        return Some((
            scale(hexval(r)?, r.len()),
            scale(hexval(g)?, g.len()),
            scale(hexval(b)?, b.len()),
        ));
    }
    if let Some(hex) = value.strip_prefix(b"#") {
        let n = match hex.len() {
            3 => 1,
            6 => 2,
            9 => 3,
            12 => 4,
            _ => return None,
        };
        return Some((
            scale(hexval(&hex[..n])?, n),
            scale(hexval(&hex[n..2 * n])?, n),
            scale(hexval(&hex[2 * n..])?, n),
        ));
    }
    None
}

fn canonicalize_params_1(params: &vte::Params, default: u16) -> u16 {
    let first = params.iter().next().map_or(0, |x| *x.first().unwrap_or(&0));
    if first == 0 {
        default
    } else {
        first
    }
}

fn canonicalize_params_2(params: &vte::Params, default1: u16, default2: u16) -> (u16, u16) {
    let mut iter = params.iter();
    let first = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let first = if first == 0 { default1 } else { first };

    let second = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let second = if second == 0 { default2 } else { second };

    (first, second)
}

fn canonicalize_params_decstbm(
    params: &vte::Params,
    size: crate::terminal::grid::Size,
) -> (u16, u16) {
    let mut iter = params.iter();
    let top = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let top = if top == 0 { 1 } else { top };

    let bottom = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let bottom = if bottom == 0 { size.rows } else { bottom };

    (top, bottom)
}
