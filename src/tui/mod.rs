pub mod animation;
pub mod blit;
pub mod dashboard_layout;
pub mod layout;
pub mod screens;
pub mod text;
pub mod theme;
pub mod tween;
pub mod widgets;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::{App, AppMode};
use crate::theme::catalog::{PaintRole, StyleRole};
use crate::theme::gradient::{paint_gradient_area, CellSelection, PaintChannel};
use crate::theme::model::ResolvedPaint;

/// Panic-safe popup dimension: clamp `desired` into `[min, avail]`, but never
/// let `min` exceed `avail` (which would make `u16::clamp` assert `min <= max`
/// and crash the whole TUI on a terminal smaller than the popup's minimum).
/// On a too-small terminal the popup just shrinks to the available space.
pub fn fit_popup(desired: u16, min: u16, avail: u16) -> u16 {
    desired.clamp(min.min(avail), avail)
}

/// The foreground a popup frame is drawn in.
///
/// `components.popup.border` is a `Paint` role, so it may carry a gradient.
/// This is only the first half of drawing one: the block renders its frame in
/// the role's solid fallback, and [`paint_popup_border`] then runs the gradient
/// over exactly the cells still carrying that colour. Every caller of this
/// function owes the popup that second pass, or a gradient theme silently
/// flattens to its first stop.
pub(crate) fn popup_border_style(
    theme: &crate::theme::model::ResolvedTheme,
    area: Rect,
) -> ratatui::style::Style {
    ratatui::style::Style::default().fg(blit::line_color(theme, PaintRole::PopupBorder, area))
}

/// The gradient pass belonging to [`popup_border_style`], run after the popup's
/// block has been rendered into `area`.
///
/// A thin wrapper so the many popup call sites read as a pair; all the
/// behaviour is in [`blit::paint_border`].
pub(crate) fn paint_popup_border(
    frame: &mut Frame,
    area: Rect,
    theme: &crate::theme::model::ResolvedTheme,
) {
    blit::paint_border(frame.buffer_mut(), area, theme, PaintRole::PopupBorder);
}

/// Carry `underlay`'s background onto `style`, keeping everything else.
///
/// A selected row is drawn in two passes: a bar in the selection role, then the
/// row's controls over it. A control role that carries no background of its own
/// left the cell on `Color::Reset` — punching a hole in the bar — and one that
/// carries a different background overwrote the bar with a foreign colour. Both
/// are wrong: the bar owns the background, the control owns the foreground.
///
/// Foreground and modifiers of `style` are never touched, and an `underlay`
/// without a background of its own leaves `style` exactly as it was.
pub(crate) fn inherit_background(style: Style, underlay: Style) -> Style {
    match underlay.bg {
        Some(bg) => style.bg(bg),
        None => style,
    }
}

/// Open a popup: clear the area it covers, then lay down its own background.
///
/// Every overlay goes through this instead of a bare `Clear`. `Clear` alone
/// only resets the cells, so a theme's `components.popup.background` could never
/// reach an overlay that draws its frame and text straight onto the reset
/// ground — which is exactly what all of them did.
///
/// The fill is an area paint, not a ring or a line, so a gradient role is
/// sampled per cell with no exclusions needed: `Clear` has already replaced
/// every cell under `area` with SSHub's own, and none of them can be a remote
/// PTY cell any more.
///
/// Under `default` the role is transparent — as every surface role is — so this
/// blanks to `Color::Reset` and the popup looks exactly as it always did.
pub(crate) fn open_popup(
    frame: &mut Frame,
    area: Rect,
    theme: &crate::theme::model::ResolvedTheme,
) {
    frame.render_widget(Clear, area);
    blit::fill_paint(frame.buffer_mut(), area, theme, PaintRole::PopupBackground);
}

/// Convert a Unix epoch timestamp to `"HH:MM:SS"` in the local timezone.
///
/// Uses libc `localtime_r` (reentrant, no allocation) so we stay
/// dependency-free beyond what the project already pulls in transitively.
pub fn format_local_time(epoch_secs: i64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time_t = epoch_secs as libc::time_t;
    // SAFETY: localtime_r is reentrant and writes into our stack-local `tm`.
    unsafe { libc::localtime_r(&time_t, &mut tm) };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// Frame entry point. Renders the UI, then backs the still-transparent cells:
/// with the theme's own `components.app.background` where it resolved to a real
/// colour, and with `semantic.canvas` behind whatever is left — unless the user
/// asked for a see-through interface, in which case nothing is backed at all.
pub fn render(frame: &mut Frame, app: &App) {
    render_with_transition_clock(frame, app, std::time::Instant::now());
}

/// [`render`] with the transition clock supplied by the caller.
///
/// `now` drives the [`FrameComposition`] captured below — the exit slide's
/// offset, the session-tab slide's progress and the regions protected from
/// theme paint — plus the splash fade's progress. Those passes must agree
/// about where a transition is, because the slide's blit and the protection of
/// what it blitted are one animation frame or they are a bug.
///
/// Every other animation still reads its own clock (`render_inner`, the popup
/// and tab slides, the toasts, the scroll chases). That is sound, and
/// deliberately left alone: none of them recomputes geometry that a later paint
/// pass depends on. This is not a deterministic whole-frame renderer, and the
/// name does not claim to be one.
///
/// Private: the only caller besides [`render`] is this module's own test child.
fn render_with_transition_clock(frame: &mut Frame, app: &App, now: std::time::Instant) {
    // Resolved once, before anything is drawn, and then reused by every pass
    // that composes this frame.
    let composition = FrameComposition::capture(app, frame.area(), now);
    // Reset the per-frame popup rect; each popup that draws sets it via
    // `popup_open_rect`, and we snapshot it afterwards for the close slide (#35).
    app.last_popup_rect.set(None);
    render_inner(frame, app, &composition);
    // While in the full-screen host view, keep a fresh snapshot so leaving it can
    // slide the session off to the right (#35). Once exited, render_inner has
    // drawn the dashboard beneath — blit the snapshot sliding away over it.
    if crate::app::is_session_mode(app.mode) {
        // Hold the snapshot still while a tab slide plays: it *is* the tab being
        // carried off, so refreshing it would feed the slide its own output.
        if app.session_tab_switch.is_none() {
            let remote_pty = app
                .active_session()
                .filter(|session| crate::session::render::shows_remote_pty(session))
                .map(|_| crate::session::render::remote_pty_rect(frame.area()));
            *app.session_snapshot.borrow_mut() = Some(crate::app::SessionSnapshot {
                buffer: frame.buffer_mut().clone(),
                remote_pty,
            });
        }
    } else {
        // Mirror of the above: keep the dashboard fresh so entering a session has
        // something to slide over instead of blank cells.
        *app.dashboard_snapshot.borrow_mut() = Some(frame.buffer_mut().clone());
        render_session_exit(frame, app, &composition);
    }
    // Snapshot the popup shown this frame, slide a fresh one in from the top,
    // and throw a just-closed one upward.
    capture_popup_snapshot(frame, app);
    render_popup_open(frame, app);
    render_popup_close(frame, app);
    apply_app_background(frame, app, &composition);
    apply_panel_selection(frame, app);
    // Fade the whole dashboard up on the way out of the intro animation, so the
    // first frame arrives rather than replacing the splash outright (#35).
    if let Some(at) = app.dashboard_at.filter(|_| app.motion_enabled()) {
        let p = tween::progress(at, SPLASH_FADE, now);
        if p < 1.0 {
            let area = frame.area();
            // The same regions the background pass protects, from the same
            // capture. This fade is armed once, when the event loop starts, and
            // only time ends it — so opening or switching to a session inside
            // its 360 ms window runs it over a frame full of remote output.
            blit::fade(
                frame.buffer_mut(),
                area,
                tween::ease_out(p),
                blit::FadeGround {
                    theme: app.theme(),
                    role: PaintRole::AppBackground,
                    paint_area: area,
                    exclusions: &composition.protected,
                },
            );
        }
    }
}

/// Back the cells no widget painted, in three deliberately separate passes.
///
/// SSHub is **opaque out of the box**; transparency is the user's explicit
/// choice, per surface. That direction is deliberate: whether a theme leaves
/// anything transparent to fill depends on the theme, so a switch asking to
/// *fill* is inert under a theme that already paints everything. Asking to
/// *release* is the question every theme can answer.
///
/// 1. A theme that resolved `components.app.background` to a real colour or a
///    gradient paints every still-`Color::Reset` cell of SSHub's **own**
///    surfaces — never the remote PTY viewport, whose cells carry the host's
///    ANSI colours and whose "unpainted" cells are the host's default
///    background, not ours.
/// 2. [`apply_pty_ground`] then owns exactly those protected regions, backing
///    the remote grid with the theme's own PTY pair.
/// 3. Whatever is *left* is filled with `semantic.canvas` — the cells a theme
///    resolving to `"terminal"` never claimed. It cannot suppress a surface the
///    theme set explicitly, because pass 1 ran first, and it excludes the
///    protected regions because pass 2 owns them. That exclusion is load-bearing
///    rather than defensive: with `transparent_session_background` on, pass 2
///    returns without writing anything, and the canvas would otherwise land on
///    the grid the user just asked to see through.
///
/// `appearance.transparent_sshub_background` needs nothing here beyond skipping
/// pass 3: `App::theme` already hands these passes a theme whose ground roles
/// resolve to `Color::Reset` (see [`ResolvedTheme::with_ground_released`]), so
/// pass 1 finds nothing to paint and the widgets have drawn no panel bodies. The
/// canvas fill is the one thing that would still put a colour down, and it is
/// exactly what the user asked to be rid of.
///
/// Every pass selects on `Color::Reset`, so none can touch a cell a widget
/// already coloured: releasing the ground does not stop a widget from drawing
/// its own border or text.
fn apply_app_background(frame: &mut Frame, app: &App, composition: &FrameComposition) {
    let theme = app.theme();
    let area = frame.area();
    let exclusions = &composition.protected;
    let transparent = app.config.appearance.transparent_sshub_background;
    let buf = frame.buffer_mut();

    if !transparent {
        match theme.paint(PaintRole::AppBackground) {
            // `"terminal"`: the theme asked for no ground of its own, and pass 3
            // backs it with the canvas instead.
            ResolvedPaint::Solid(Color::Reset) => {}
            ResolvedPaint::Solid(color) => {
                fill_reset_background(buf, area, *color, exclusions);
            }
            ResolvedPaint::Gradient(_) => {
                if let Some(gradient) = theme.paint_gradient(PaintRole::AppBackground) {
                    paint_gradient_area(
                        buf,
                        area,
                        gradient,
                        PaintChannel::Background,
                        CellSelection::Matching(Color::Reset),
                        exclusions,
                    );
                }
            }
        }
    }

    apply_pty_ground(buf, app, app.base_theme(), exclusions);

    if !transparent {
        fill_reset_background(buf, area, theme.semantic().canvas, exclusions);
    }
}

/// Back the remote grid with the theme's own ground, foreground included.
///
/// Runs over `protected` — the resting viewport plus the travelling bands of an
/// exit or session-tab slide — so a session in transit is backed exactly like
/// one at rest.
///
/// The pair comes from `semantic.pty_background` / `semantic.pty_foreground`,
/// which `default` defines as references to `background` / `text`: a theme that
/// paints its own ground therefore paints it here too. Where a theme leaves its
/// ground to the emulator, the canvas and the plain text colour stand in, so
/// the grid is opaque out of the box under every theme.
///
/// `appearance.transparent_session_background` is the user overriding all of
/// that and handing the grid straight back to the emulator — the surface a
/// terminal wallpaper shows through best, and the reason the switch exists
/// separately from the one for SSHub's own surfaces.
fn apply_pty_ground(
    buf: &mut Buffer,
    app: &App,
    theme: &crate::theme::model::ResolvedTheme,
    protected: &[Rect],
) {
    if app.config.appearance.transparent_session_background {
        return;
    }
    let Some(ground) = PtyGround::of(theme) else {
        return;
    };

    for region in protected {
        fill_reset_pair(buf, *region, ground);
    }
}

/// The opaque `(background, foreground)` pair painted under the remote grid.
///
/// A named type rather than a bare `(Color, Color)`, because the two are not
/// two colours that happen to travel together: the pair *is* the invariant. Its
/// only constructor establishes it, so no caller can assemble a half-usable one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PtyGround {
    background: Color,
    foreground: Color,
}

impl PtyGround {
    /// Resolve the pair for `theme`, substituting a usable value for either
    /// channel the theme left to the emulator.
    ///
    /// `"terminal"` is legal for every semantic slot but `background`, so a
    /// theme may pair a painted ground with an emulator-owned foreground — and
    /// taking that at face value re-opens the reported bug, because a written
    /// background over a `Reset` foreground is the emulator's own colour on our
    /// ground. Each channel therefore falls back on its own: the canvas for the
    /// ground, the plain text colour for what sits on it.
    ///
    /// `None` when even a fallback resolves to `Color::Reset` — a theme may set
    /// `text = "terminal"` too, and there is no honest colour left to invent.
    /// Writing the half that *is* usable would be the reported bug all over
    /// again, so nothing is written and the grid stays the emulator's.
    fn of(theme: &crate::theme::model::ResolvedTheme) -> Option<Self> {
        let semantic = theme.semantic();
        let usable = |colour: Color, fallback: Color| {
            let chosen = if colour == Color::Reset {
                fallback
            } else {
                colour
            };
            (chosen != Color::Reset).then_some(chosen)
        };
        Some(Self {
            background: usable(semantic.pty_background, semantic.canvas)?,
            foreground: usable(semantic.pty_foreground, semantic.text)?,
        })
    }
}

/// Write a `(background, foreground)` pair into the channels of `area` that are
/// still `Color::Reset`, testing each channel on its own.
///
/// The channels are always offered together, and that is the point. Filling only
/// the background leaves the remote's default foreground to the emulator, which
/// then writes its own — near-white on a light theme's cream ground. It also
/// breaks reverse video: a `REVERSED` cell with both channels at `Reset` would
/// swap our ground against a foreground that was never defined.
///
/// A cell the remote coloured explicitly keeps that colour in the channel it
/// set, and receives the pair's value only in the channel it left alone.
fn fill_reset_pair(buf: &mut Buffer, area: Rect, ground: PtyGround) {
    let target = area.intersection(buf.area);
    for y in target.y..target.bottom() {
        for x in target.x..target.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                if cell.bg == Color::Reset {
                    cell.bg = ground.background;
                }
                if cell.fg == Color::Reset {
                    cell.fg = ground.foreground;
                }
            }
        }
    }
}

/// Everything about this frame's composition that more than one pass must agree
/// on, resolved once from a single clock reading.
///
/// A frame is composed, not simply drawn: the dashboard goes down, a session
/// snapshot may be blitted over it, and only then do the background pass and the
/// splash fade run. Each of those used to re-read the wall clock, so the slide's
/// blit and the region protected from theme paint could be a millisecond — and
/// therefore a column or more — apart, leaving the leading remote columns
/// exposed. At the end of the slide they could disagree about whether it was
/// still playing at all.
struct FrameComposition {
    /// How far the exit slide has carried its snapshot, if one is playing.
    exit_offset: Option<u16>,
    /// How far the enter slide still has to carry the session in, if one is
    /// playing. Read by `render_session_enter` so the blit and the ownership
    /// derived from it are one animation frame.
    enter_offset: Option<u16>,
    /// Eased progress of a session-tab slide, if its snapshot is still visible.
    tab_slide_progress: Option<f32>,
    /// Every region of the composed frame that carries remote output, and which
    /// no theme paint or fade may touch.
    protected: Vec<Rect>,
}

impl FrameComposition {
    fn capture(app: &App, area: Rect, now: std::time::Instant) -> Self {
        let mut protected = Vec::new();
        let tab_slide_progress = will_render_session_tab_slide(app)
            .then(|| session_tab_slide_progress(app, now))
            .flatten();
        let enter_offset = session_enter_offset(app, area, now);

        // 1. The live viewport, while the session view *is* the frame. Both the
        //    rect and the decision come from `session::render`, so the protected
        //    region cannot drift from what the `tui_term` widget really covers.
        //    The connecting spinner and the failure screen occupy the same rows
        //    but are SSHub's own chrome, and a theme is allowed to back them.
        //    During a tab slide the live layer is shifted, so the transition's
        //    translated ownership regions below replace this resting rect.
        //
        //    An enter slide shifts it too: the session arrives from the right
        //    while the dashboard is restored on its left, so the resting rect
        //    reaches over cells that are SSHub's own this frame. Protecting
        //    those would hand the grid's ground to the dashboard.
        if tab_slide_progress.is_none() && shows_session_view(app) {
            if let Some(session) = app.active_session() {
                if crate::session::render::shows_remote_pty(session) {
                    let resting = crate::session::render::remote_pty_rect(area);
                    match enter_offset {
                        Some(offset) => {
                            protected.extend(translated_region(resting, i32::from(offset), area))
                        }
                        None => protected.push(resting),
                    }
                }
            }
        }

        // 2. The travelling band of an exit slide. That frame's *mode* is
        //    already the dashboard, but `render_session_exit` has just blitted a
        //    still-visible session snapshot over it — including its remote
        //    cells, whose unwritten backgrounds are `Color::Reset` and would
        //    otherwise be filled.
        let exit_offset = session_exit_offset(app, area, now);
        if let Some(offset) = exit_offset {
            protected.extend(exit_snapshot_region(app, area, offset));
        }

        // 3. A session-tab slide protects only the shifted pieces that really
        // carry remote output. Connecting/failed layers are SSHub chrome and
        // must still receive the app background while travelling.
        if let Some(progress) = tab_slide_progress {
            protected.extend(session_tab_slide_regions(app, area, progress));
        }

        Self {
            exit_offset,
            enter_offset,
            tab_slide_progress,
            protected,
        }
    }
}

/// How far an enter slide still has to travel, or `None` when none is playing.
///
/// Every condition `render_session_enter` declines on is repeated here, because
/// the offset and the ownership derived from it have to agree exactly: a slide
/// that is captured but not blitted protects cells nothing owns, and one that is
/// blitted but not captured paints over remote output.
fn session_enter_offset(app: &App, area: Rect, now: std::time::Instant) -> Option<u16> {
    if !app.motion_enabled() || session_behind_picker(app) || !shows_session_view(app) {
        return None;
    }
    let at = app.session_enter_at?;
    let p = tween::progress(at, SESSION_ANIM, now);
    if p >= 1.0 {
        return None;
    }
    // No usable dashboard behind it means no slide at all — see the reasoning in
    // `render_session_enter`.
    if !app
        .dashboard_snapshot
        .borrow()
        .as_ref()
        .is_some_and(|b| b.area == area)
    {
        return None;
    }
    // Off starts a full screen-width to the right (fully off) and eases to 0.
    let off = ((1.0 - tween::ease_out(p)) * area.width as f32).round() as u16;
    (off > 0).then_some(off)
}

fn session_tab_slide_progress(app: &App, now: std::time::Instant) -> Option<f32> {
    if !app.motion_enabled() || app.session_snapshot.borrow().is_none() {
        return None;
    }
    let switch = app.session_tab_switch?;
    let progress = tween::progress(switch.at, TAB_ANIM, now);
    (progress < 1.0).then(|| tween::ease_out(progress))
}

fn session_tab_slide_regions(app: &App, frame_area: Rect, progress: f32) -> Vec<Rect> {
    let Some(switch) = app.session_tab_switch else {
        return Vec::new();
    };
    let viewport = crate::session::render::remote_pty_rect(frame_area);
    let width = viewport.width as f32;
    let direction = switch.dir as f32;
    let mut regions = Vec::with_capacity(2);

    if let Some(remote) = app
        .session_snapshot
        .borrow()
        .as_ref()
        .and_then(|snapshot| snapshot.remote_pty)
    {
        let source = remote.intersection(viewport);
        if let Some(visible) = translated_region(
            source,
            (-direction * progress * width).round() as i32,
            viewport,
        ) {
            regions.push(visible);
        }
    }

    if app
        .active_session()
        .is_some_and(crate::session::render::shows_remote_pty)
    {
        if let Some(visible) = translated_region(
            viewport,
            (direction * (1.0 - progress) * width).round() as i32,
            viewport,
        ) {
            regions.push(visible);
        }
    }

    regions
}

fn translated_region(source: Rect, offset_x: i32, clip: Rect) -> Option<Rect> {
    if source.is_empty() {
        return None;
    }
    let left = i32::from(source.x) + offset_x;
    let right = left + i32::from(source.width);
    let clipped_left = left.max(i32::from(clip.x));
    let clipped_right = right.min(i32::from(clip.right()));
    if clipped_left >= clipped_right {
        return None;
    }
    Some(Rect::new(
        clipped_left as u16,
        source.y.max(clip.y),
        (clipped_right - clipped_left) as u16,
        source.bottom().min(clip.bottom()) - source.y.max(clip.y),
    ))
}

/// Where the exit snapshot's remote cells land in *this* frame.
///
/// Derived from the snapshot's own geometry, not from the current frame's. The
/// terminal can be resized while a slide plays (`crate::run_terminal_loop`
/// resizes every session live and does not drop the snapshot), so a snapshot
/// taken at 24 rows can be sliding out of a 20-row frame: rows that were the old
/// PTY body then land on rows that are now footer or dashboard. Those cells are
/// still the host's output. The rectangle is therefore taken from the snapshot,
/// translated by the exact blit offset, and clipped to what the frame shows —
/// which also keeps a *grown* terminal from having rows the snapshot never
/// reached carved out of the dashboard.
///
fn exit_snapshot_region(app: &App, area: Rect, offset: u16) -> Option<Rect> {
    let snapshot = app.session_snapshot.borrow();
    let source = snapshot.as_ref()?.remote_pty?;
    // The slide is horizontal, so the rows stay put and only the left edge
    // moves: everything left of the offset is the dashboard being revealed, and
    // must still be painted.
    let travelled = Rect::new(
        source.x.saturating_add(offset),
        source.y,
        source.width,
        source.height,
    );
    let visible = travelled.intersection(area);
    (!visible.is_empty()).then_some(visible)
}

/// How far the session-exit slide has carried its snapshot to the right at
/// `now`, or `None` when no slide is playing.
///
/// Read exactly once per frame, by [`FrameComposition::capture`].
fn session_exit_offset(app: &App, area: Rect, now: std::time::Instant) -> Option<u16> {
    if !app.motion_enabled() {
        return None;
    }
    let at = app.session_exit_at?;
    if app.session_snapshot.borrow().is_none() {
        return None;
    }
    let p = tween::progress(at, SESSION_ANIM, now);
    if p >= 1.0 {
        return None;
    }
    // Off eases from 0 to a full screen-width, carrying the session off the right.
    Some((tween::ease_out(p) * area.width as f32).round() as u16)
}

/// Give every still-transparent cell of `area` the background `color`, leaving
/// `exclusions` untouched.
fn fill_reset_background(buf: &mut Buffer, area: Rect, color: Color, exclusions: &[Rect]) {
    let target = area.intersection(buf.area);
    for y in target.y..target.bottom() {
        for x in target.x..target.right() {
            if exclusions.iter().any(|rect| rect.contains((x, y).into())) {
                continue;
            }
            if let Some(cell) = buf.cell_mut((x, y)) {
                if cell.bg == Color::Reset {
                    cell.bg = color;
                }
            }
        }
    }
}

/// Highlight the zoomed-panel text selection (issue #18) by reversing the
/// selected cells, and extract the selected text into `app.panel_sel_text` for
/// copy-on-release. Terminal-style stream selection over the dashboard body.
fn apply_panel_selection(frame: &mut Frame, app: &App) {
    if !app.panel_zoomed {
        return;
    }
    let Some(sel) = app.panel_sel else {
        app.panel_sel_text.borrow_mut().clear();
        return;
    };
    let body = dashboard_layout::dashboard_layout_zoomed(frame.area(), app.ui_zoom).body;
    // Order anchor/pointer in reading (row-major) order.
    let (a, b) = (sel.anchor, sel.cur);
    let (start, end) = if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    };
    let last_row = body.y + body.height.saturating_sub(1);
    let last_col = body.x + body.width.saturating_sub(1);
    let y0 = start.1.max(body.y);
    let y1 = end.1.min(last_row);
    if y0 > y1 {
        app.panel_sel_text.borrow_mut().clear();
        return;
    }
    let buf = frame.buffer_mut();
    let mut text = String::new();
    for row in y0..=y1 {
        let x_from = if row == start.1 { start.0 } else { body.x }.max(body.x);
        let x_to = if row == end.1 { end.0 } else { last_col }.min(last_col);
        let mut line = String::new();
        if x_from <= x_to {
            for col in x_from..=x_to {
                if let Some(cell) = buf.cell_mut((col, row)) {
                    cell.modifier.insert(Modifier::REVERSED);
                    line.push_str(cell.symbol());
                }
            }
        }
        if row != y0 {
            text.push('\n');
        }
        text.push_str(line.trim_end());
    }
    *app.panel_sel_text.borrow_mut() = text;
}

/// Whether the picker is floating over a session rather than the dashboard.
fn session_behind_picker(app: &App) -> bool {
    app.session_picker_over_session()
}

/// Whether this frame is the full-screen session view rather than the
/// dashboard. Shared with the app-background pass, so the region it protects
/// cannot be claimed on a frame that never drew a session.
fn shows_session_view(app: &App) -> bool {
    app.session_is_rendered()
}

/// Whether this frame takes the exact branch that blits a session-tab slide.
fn will_render_session_tab_slide(app: &App) -> bool {
    shows_session_view(app) && !session_behind_picker(app)
}

fn render_inner(frame: &mut Frame, app: &App, composition: &FrameComposition) {
    let session_behind_picker = session_behind_picker(app);

    // Embedded session takes over the whole frame — no dashboard chrome.
    if shows_session_view(app) {
        crate::session::render::render(frame, app);
        // Slide the freshly-connected session in from the right (#35). Skipped
        // for the picker-over-session case (no fresh connect happening).
        if !session_behind_picker {
            render_session_enter(frame, app, composition);
        }
        if will_render_session_tab_slide(app) {
            render_session_tab_slide(frame, app, composition);
        }
        if app.mode == AppMode::SessionPicker {
            // Snapshot the session underneath before the picker draws, so its
            // drop-in can restore what it covers (#35) — the same contract the
            // dashboard branch honours for every other popup.
            if app.motion_enabled() {
                *app.popup_backdrop.borrow_mut() = Some(frame.buffer_mut().clone());
            }
            screens::session_picker::render(frame, app);
        }
        if app.mode == AppMode::SnippetPicker {
            if app.motion_enabled() {
                *app.popup_backdrop.borrow_mut() = Some(frame.buffer_mut().clone());
            }
            screens::snippet_picker::render(frame, app);
        }
        return;
    }

    // ── Dashboard chrome (shared across all tabs) ─────────────
    let area = frame.area();
    let areas = dashboard_layout::dashboard_layout_zoomed(area, app.ui_zoom);

    // Header stats
    let theme = app.theme();
    let [total, online, slow, down] = app.header_stats_advance(compute_header_stats(app));
    let clock = format_utc_clock();
    widgets::header::render_header(
        frame,
        areas.header,
        widgets::header::HeaderStats {
            host_count: total,
            online,
            slow,
            down,
            clock: &clock,
        },
        theme,
    );

    // Open embedded sessions — visible strip on the top header row so
    // background SSH tabs aren't hidden behind a footer hint.
    let session_chips = build_session_chips(app);
    // Cycling session tabs from the dashboard used to change the highlighted
    // chip with no motion at all, while the same keys inside the full-screen
    // view slide it. Both now read the same travel state (#35).
    let strip_travel = crate::session::render::highlight_travel(app).and_then(|p| {
        let sw = app.session_tab_switch?;
        Some(widgets::header::StripTravel {
            from: sw.from,
            to: app.active_session?,
            p,
        })
    });
    widgets::header::render_session_strip(frame, areas.header, &session_chips, strip_travel, theme);

    // Horizontal rule 1
    let rule1 = row_in(area, areas.header.y + areas.header.height);
    widgets::footer::render_hrule(frame, rule1, false, theme, PaintRole::HeaderSeparator);

    // Tab bar
    let scope_path = "~/.config/sshub";
    widgets::tab_bar::render_tab_bar(frame, areas.tab_bar, app.active_tab + 1, scope_path, theme);

    // Horizontal rule 2
    let rule2 = row_in(area, areas.tab_bar.y + areas.tab_bar.height);
    widgets::footer::render_hrule(frame, rule2, false, theme, PaintRole::TabsSeparator);

    // ── Tab body dispatch (with slide animation, #35) ─────────
    let now = std::time::Instant::now();
    let sliding = app
        .tab_switch
        .filter(|s| app.motion_enabled() && now.saturating_duration_since(s.at) < TAB_ANIM);
    if let Some(sw) = sliding {
        render_tab_slide(frame, &areas, app, sw, now);
    } else {
        render_tab_body(frame, app.active_tab, &areas, app);
    }

    // ── Broadcast mode (#3): docked live panel floats over the dashboard ──
    // While a broadcast runs it lives in the bottom-right as a floating panel
    // (or full-body when zoomed + focused). Other panels are not moved, just
    // covered. The wizard overlays are handled in the mode match below.
    if let Some(bc) = app.broadcast.as_ref() {
        let body = dashboard_layout::dashboard_layout_zoomed(frame.area(), app.ui_zoom).body;
        if app.panel_zoomed && app.focused_panel == crate::app::PanelId::Broadcast {
            screens::broadcast::render_broadcast_zoomed(frame, body, app);
        } else {
            let rect = match bc.anim {
                Some(a) if app.motion_enabled() => a.rect_at(std::time::Instant::now()),
                // Reduced motion (or no anim): sit at the resting docked rect.
                _ => screens::broadcast::docked_rect(body),
            };
            let focused = app.focused_panel == crate::app::PanelId::Broadcast;
            screens::broadcast::render_broadcast_panel(frame, rect, app, focused);
        }
    }
    // Error toasts stack above the docked panel (and can outlive it), so draw
    // them whenever any exist — not only while the panel is present.
    if !app.broadcast_toasts.is_empty() {
        let body = dashboard_layout::dashboard_layout_zoomed(frame.area(), app.ui_zoom).body;
        screens::broadcast::render_broadcast_toasts(frame, body, app);
    }

    // Horizontal rule 3: above footer (bold)
    let rule3 = row_in(area, areas.footer.y.saturating_sub(1));
    widgets::footer::render_hrule(frame, rule3, true, theme, PaintRole::FooterSeparator);

    // Footer keybinds (tab-specific)
    let (keybinds, pinned) = footer_keybinds(app);
    widgets::footer::render_footer(frame, areas.footer, &keybinds, pinned, theme);

    // The status bar that used to print `host_notice` is gone (dead code, #58),
    // so this toast is the *only* surface left: gating it on `panel_zoomed` made
    // every dashboard notice invisible — push-key errors, import/export results,
    // SFTP failures — and a silent failure reads as a broken keybind.
    if let Some(notice) = &app.host_notice {
        render_notice_toast(frame, areas.footer, notice, app);
    }

    // ── Overlay popups ─────────────────────────────────────────
    // Snapshot the dashboard (no popup yet) so the open slide can restore what's
    // behind the popup and let it drop in from off the top of the screen (#35).
    if app.motion_enabled() && crate::app::is_overlay_mode(app.mode) {
        *app.popup_backdrop.borrow_mut() = Some(frame.buffer_mut().clone());
    }
    match app.mode {
        AppMode::Palette => {
            screens::palette::render_palette(
                frame,
                app,
                &app.palette_query,
                &app.hosts,
                &app.palette_results,
                app.palette_selected,
                app.palette_adhoc.as_ref(),
            );
        }
        AppMode::HostForm => render_form_popup(frame, app, FormKind::Host),
        AppMode::FieldPicker => {
            render_form_popup(frame, app, FormKind::Host);
            screens::field_picker::render_field_picker(frame, app);
        }
        AppMode::IdentityForm => render_form_popup(frame, app, FormKind::Identity),
        AppMode::KeygenForm => render_form_popup(frame, app, FormKind::Keygen),
        AppMode::GroupManage => screens::group_manage::render_group_manage_popup(frame, app),
        AppMode::GroupForm => {
            // Keep the group list behind the form when it was opened from the
            // group-management popup, for context.
            if app.group_form.as_ref().is_some_and(|f| f.return_to_manage) {
                screens::group_manage::render_group_manage_popup(frame, app);
            }
            render_form_popup(frame, app, FormKind::Group);
        }
        AppMode::GroupFieldPicker => {
            if app.group_form.as_ref().is_some_and(|f| f.return_to_manage) {
                screens::group_manage::render_group_manage_popup(frame, app);
            }
            render_form_popup(frame, app, FormKind::Group);
            screens::group_form::render_group_field_picker(frame, app);
        }
        AppMode::TagFilter => screens::tag_filter::render(frame, app),
        AppMode::TunnelForm => screens::tunnels::render_tunnel_form(frame, app),
        AppMode::TunnelHostPicker => {
            screens::tunnels::render_tunnel_form(frame, app);
            screens::tunnels::render_tunnel_host_picker(frame, app);
        }
        AppMode::SessionPicker => screens::session_picker::render(frame, app),
        AppMode::PushKeyHostPicker => screens::push_key_pickers::render_host_picker(frame, app),
        AppMode::PushKeyIdentityPicker => {
            screens::push_key_pickers::render_identity_picker(frame, app)
        }
        AppMode::ConfirmDiscard => {
            if app.host_form.is_some() {
                render_form_popup(frame, app, FormKind::Host);
            } else if app.identity_form.is_some() {
                render_form_popup(frame, app, FormKind::Identity);
            } else if app.tunnel_form.is_some() {
                screens::tunnels::render_tunnel_form(frame, app);
            } else if app.snippet_form.is_some() {
                screens::snippet_manage::render_snippet_manage_popup(frame, app);
                screens::snippet_form::render_snippet_form(frame, app);
            }
            render_confirm_discard_popup(frame, app);
        }
        AppMode::ConfirmDelete => render_confirm_delete_popup(frame, app),
        AppMode::Help => render_help_popup(frame, app),
        AppMode::KeybindEditor => screens::keybind_editor::render_keybind_editor(frame, app),
        AppMode::Settings => screens::settings::render_settings(frame, app),
        AppMode::ThemePicker => screens::theme_picker::render(frame, app),
        AppMode::TunnelReconnectSettings => {
            screens::tunnel_reconnect::render_tunnel_reconnect_settings(frame, app);
        }
        AppMode::ConfirmQuit => render_confirm_quit_popup(frame, app),
        AppMode::ImportPrompt => render_import_prompt_popup(frame, app),
        AppMode::SftpPrompt => render_sftp_prompt_popup(frame, app),
        AppMode::BroadcastPickTarget => screens::broadcast::render_pick_target(frame, app),
        AppMode::BroadcastCommand => screens::broadcast::render_command_prompt(frame, app),
        AppMode::BroadcastPreview => screens::broadcast::render_preview(frame, app),
        AppMode::Notice => render_notice_popup(frame, app),
        AppMode::KnownHosts => screens::known_hosts::render_known_hosts(frame, app),
        AppMode::LogBrowser => screens::log_browser::render(frame, app),
        AppMode::SnippetManage => screens::snippet_manage::render_snippet_manage_popup(frame, app),
        AppMode::SnippetForm => {
            // Keep the snippet list behind the form for context.
            screens::snippet_manage::render_snippet_manage_popup(frame, app);
            screens::snippet_form::render_snippet_form(frame, app);
        }
        _ => {}
    }
}

/// Modal message popup (`AppMode::Notice`) — e.g. an SFTP connection error.
/// Text comes from `App::notice_popup`; any key dismisses it.
fn render_notice_popup(frame: &mut Frame, app: &App) {
    let Some(message) = app.notice_popup.as_ref() else {
        return;
    };
    let hint = "press any key to dismiss";

    let area = frame.area();
    let popup_width = 60u16.min(area.width).max(20.min(area.width));
    // Rough wrapped-line count so the box grows with the message.
    let inner_w = popup_width.saturating_sub(2).max(1) as usize;
    let msg_lines = message
        .split('\n')
        .map(|l| (l.chars().count() / inner_w) + 1)
        .sum::<usize>()
        .max(1);
    let popup_height = ((msg_lines as u16) + 4)
        .min(area.height)
        .max(5.min(area.height));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup_area = crate::tui::popup_open_rect(popup_area, app);
    let theme = app.theme();
    let error = theme.style(StyleRole::PopupError);

    open_popup(frame, popup_area, theme);
    frame.render_widget(
        Paragraph::new(format!("{message}\n\n{hint}"))
            .wrap(Wrap { trim: false })
            .style(error)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled(" Connection failed ", error))
                    .border_style(error),
            ),
        popup_area,
    );
}

fn render_sftp_prompt_popup(frame: &mut Frame, app: &App) {
    let Some(prompt) = app.sftp_prompt.as_ref() else {
        return;
    };
    use crate::app::SftpPromptKind;

    let (title, label) = match prompt.kind {
        SftpPromptKind::Mkdir => (" New folder ", "New folder name:"),
        SftpPromptKind::Rename => (" Rename ", "Rename to:"),
        SftpPromptKind::Chmod => (" Permissions ", "Permissions (octal, e.g. 755):"),
    };

    let area = frame.area();
    let popup_width = (area.width * 70 / 100).max(40).min(area.width);
    let popup_height = if prompt.error.is_some() { 9 } else { 7 }.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let theme = app.theme();
    let mut lines = vec![
        ratatui::text::Line::from(Span::styled(label, theme.style(StyleRole::TextPrimary))),
        ratatui::text::Line::from(Span::styled(
            crate::text_input::with_cursor(&prompt.value, prompt.cursor),
            theme.style(StyleRole::FormInput),
        )),
        ratatui::text::Line::from(""),
    ];
    if let Some(err) = &prompt.error {
        lines.push(ratatui::text::Line::from(Span::styled(
            format!("\u{2717} {err}"),
            theme.style(StyleRole::PopupError),
        )));
        lines.push(ratatui::text::Line::from(""));
    }
    lines.push(ratatui::text::Line::from(Span::styled(
        "Enter: confirm  \u{2502}  Esc: cancel",
        theme.style(StyleRole::PopupHint),
    )));

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

    open_popup(frame, popup_area, theme);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title, theme.style(StyleRole::PopupTitle)))
                .border_style(popup_border_style(theme, popup_area)),
        ),
        popup_area,
    );
    paint_popup_border(frame, popup_area, theme);
}

fn render_import_prompt_popup(frame: &mut Frame, app: &App) {
    let Some(prompt) = app.import_prompt.as_ref() else {
        return;
    };

    let area = frame.area();
    let popup_width = (area.width * 80 / 100).max(50).min(area.width);
    let popup_height = if prompt.error.is_some() { 10 } else { 8 }.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let theme = app.theme();
    let mut lines = vec![
        ratatui::text::Line::from(Span::styled(
            "Path to Termius export folder (contains L00t.csv, ssh_keys/):",
            theme.style(StyleRole::TextPrimary),
        )),
        ratatui::text::Line::from(Span::styled(
            crate::text_input::with_cursor(&prompt.path, prompt.cursor),
            theme.style(StyleRole::FormInput),
        )),
        ratatui::text::Line::from(""),
    ];
    if let Some(err) = &prompt.error {
        lines.push(ratatui::text::Line::from(Span::styled(
            format!("\u{2717} {err}"),
            theme.style(StyleRole::PopupError),
        )));
        lines.push(ratatui::text::Line::from(""));
    }
    lines.push(ratatui::text::Line::from(Span::styled(
        "Enter: import  \u{2502}  Esc: cancel",
        theme.style(StyleRole::PopupHint),
    )));

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

    open_popup(frame, popup_area, theme);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " Import from Termius ",
                    theme.style(StyleRole::PopupTitle),
                ))
                .border_style(popup_border_style(theme, popup_area)),
        ),
        popup_area,
    );
    paint_popup_border(frame, popup_area, theme);
}

/// A one-row rect at `y`, or a zero-height rect when `y` falls outside
/// `area` (tiny terminals) — rendering helpers skip zero-height rects.
fn row_in(area: Rect, y: u16) -> Rect {
    if y >= area.y && y < area.y + area.height {
        Rect::new(area.x, y, area.width, 1)
    } else {
        Rect::new(area.x, area.y, area.width, 0)
    }
}

fn build_session_chips(app: &App) -> Vec<widgets::header::SessionChip> {
    use crate::session::SessionPhase;
    use widgets::header::{SessionChip, SessionDot};

    app.sessions
        .iter()
        .enumerate()
        .map(|(i, s)| SessionChip {
            name: s.display_name.clone(),
            dot: match s.phase {
                SessionPhase::Connecting { .. } => SessionDot::Connecting,
                SessionPhase::Running { .. } => SessionDot::Running,
                SessionPhase::Exited { .. } => SessionDot::Exited,
            },
            active: app.active_session == Some(i),
        })
        .collect()
}

fn compute_header_stats(app: &App) -> [usize; 4] {
    use crate::ping::{classify_ping, PingClass};

    let total = app.hosts.len();
    let mut online = 0usize;
    let mut slow = 0usize;
    let mut down = 0usize;
    for h in &app.hosts {
        match classify_ping(app.ping_data.get(h.name()).map(|v| v.as_slice())) {
            PingClass::Online => online += 1,
            PingClass::Slow => slow += 1,
            PingClass::Unreachable => down += 1,
            PingClass::Unknown => {}
        }
    }
    [total, online, slow, down]
}

/// The footer's pairs, plus how many trailing ones must never be dropped.
fn footer_keybinds(app: &App) -> (Vec<(String, &'static str)>, usize) {
    let mut binds: Vec<(String, &'static str)> = match app.active_tab {
        0 => vec![
            ("\u{2191}\u{2193}".into(), "select"),
            ("\u{21b5}".into(), "connect"),
            ("/".into(), "search"),
            ("#".into(), "tags"),
            ("a".into(), "add"),
            ("e".into(), "edit"),
            ("d".into(), "del"),
            ("P".into(), "push key"),
            ("+/-".into(), "zoom"),
            ("\u{2423}".into(), "fold"),
            ("G".into(), "groups"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        1 => vec![
            ("\u{2191}\u{2193}".into(), "select"),
            ("\u{21b5}".into(), "enter/connect"),
            ("\u{21c6}".into(), "focus"),
            // Once the left pane points at a second server, the way back to the
            // local filesystem is the thing that needs saying: `o` only leads
            // further away, and nothing else on screen mentions `O`.
            if app.sftp.as_ref().is_some_and(|s| s.left_is_remote()) {
                ("O".into(), "local")
            } else {
                ("o".into(), "2nd host")
            },
            ("\u{2190}".into(), "download"),
            ("\u{2192}".into(), "upload"),
            ("c".into(), "run"),
            ("u".into(), "unstage"),
            ("d".into(), "delete"),
            ("n".into(), "new dir"),
            ("R".into(), "rename"),
            ("M".into(), "chmod"),
            ("r".into(), "refresh"),
            ("s".into(), "ssh"),
            (
                ".".into(),
                if app.sftp_show_hidden {
                    "hide dotfiles"
                } else {
                    "show hidden"
                },
            ),
            ("/".into(), "search"),
            ("Esc".into(), "back"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        2 => vec![
            ("\u{2191}\u{2193}".into(), "select"),
            ("\u{21b5}".into(), "start/stop"),
            ("a".into(), "new tunnel"),
            ("e".into(), "edit"),
            ("d".into(), "delete"),
            ("x".into(), "kill"),
            ("R".into(), "reconnect"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        3 => vec![
            ("\u{2191}\u{2193}\u{2190}\u{2192}".into(), "move"),
            ("[ ]".into(), "columns"),
            ("a".into(), "add"),
            ("g".into(), "generate"),
            ("e".into(), "edit"),
            ("d".into(), "delete"),
            ("p/r".into(), "agent +/-"),
            ("P".into(), "push key"),
            ("H".into(), "known hosts"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        4 => vec![
            ("\u{2191}\u{2193}".into(), "select"),
            ("f".into(), "filter"),
            ("r".into(), "range"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        _ => vec![("q".into(), "quit")],
    };
    // Issue #18: surface the panel-zoom hint once a panel is focused/zoomed.
    if app.active_tab == 0
        && (app.panel_zoomed || app.focused_panel != crate::app::PanelId::default())
    {
        binds.push(("z".into(), if app.panel_zoomed { "unzoom" } else { "zoom" }));
    }
    if app.active_tab == 0 && app.panel_zoomed && app.focused_panel != crate::app::PanelId::Hosts {
        let selectable = matches!(
            app.focused_panel,
            crate::app::PanelId::Ping | crate::app::PanelId::Recent
        );
        binds.push((
            "\u{2191}\u{2193}".into(),
            if selectable { "select" } else { "scroll" },
        ));
        if selectable {
            binds.push(("\u{21b5}".into(), "connect"));
        }
    }
    if app.active_tab == 0 && app.panel_zoomed {
        binds.push(("drag".into(), "copy"));
    }
    // Broadcast mode (#3): running panel gets a cancel hint (and a zoom hint
    // once focused); an active wizard step gets next/cancel.
    if app.broadcast.is_some() {
        binds.push(("x".into(), "cancel"));
        if app.focused_panel == crate::app::PanelId::Broadcast {
            binds.push(("z".into(), "zoom"));
        }
    } else if !app.broadcast_toasts.is_empty() {
        binds.push(("x".into(), "clear errors"));
    }
    if matches!(
        app.mode,
        AppMode::BroadcastPickTarget | AppMode::BroadcastCommand | AppMode::BroadcastPreview
    ) {
        binds.push(("\u{21b5}".into(), "next"));
        binds.push(("Esc".into(), "cancel"));
    }
    if !app.sessions.is_empty() {
        binds.extend(app.config.keybinds.session_footer_hints());
    }

    // Move the pairs that say how to get out, or back into a session, to the end
    // and report how many there are, because the footer pins its tail when the
    // row does not fit. Every conditional block above (panel zoom, broadcast,
    // the session hints) otherwise pushes `? help` and `q quit` into the middle,
    // which is exactly where truncation eats them.
    const PINNED_LABELS: [&str; 3] = ["resume", "help", "quit"];
    let mut pinned: Vec<(String, &'static str)> = Vec::new();
    for label in PINNED_LABELS {
        if let Some(i) = binds.iter().position(|(_, l)| *l == label) {
            pinned.push(binds.remove(i));
        }
    }
    let pinned_len = pinned.len();
    binds.extend(pinned);
    (binds, pinned_len)
}

/// Draw a transient notice (issue #18) as a floating chip right-aligned on the
/// row *above* the footer keybinds. The dashboard's only notice surface — the
/// status bar it replaced is gone — so it renders zoomed or not. Sits above the
/// hints so it never clips them.
fn render_notice_toast(frame: &mut Frame, footer: Rect, notice: &str, app: &App) {
    let label = format!(" {notice} ");
    let w = label.chars().count() as u16;
    if footer.width < w || footer.y == 0 {
        return;
    }
    let rest_x = footer.x + footer.width - w;
    // Ride in from off the right edge (#35), like the broadcast toasts. Travel
    // the distance from the resting slot to the screen edge, so the toast is
    // fully off-screen at p == 0 and `set_string` clips whatever hangs over.
    let off = app
        .host_notice_at
        .filter(|_| app.motion_enabled())
        .map(|at| {
            let p = tween::progress(at, crate::broadcast::TOAST_ANIM, std::time::Instant::now());
            ((1.0 - tween::ease_out(p)) * frame.area().right().saturating_sub(rest_x) as f32)
                .round() as u16
        })
        .unwrap_or(0);
    let x = rest_x + off;
    let y = footer.y - 1;
    let style = app.theme().style(StyleRole::StatusBarToast);
    frame.buffer_mut().set_string(x, y, &label, style);
}

/// Snapshot the popup drawn this frame (opaque cells at `last_popup_rect`) into
/// `app.popup_snapshot`, so it can be thrown upward when the popup later closes
/// (#35). No-op when the current mode draws no popup.
fn capture_popup_snapshot(frame: &mut Frame, app: &App) {
    if !crate::app::is_overlay_mode(app.mode) {
        return;
    }
    let Some(rect) = app.last_popup_rect.get() else {
        return;
    };
    let rect = rect.intersection(frame.area());
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    let mut snap = Buffer::empty(rect);
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            if let (Some(src), Some(dst)) = (buf.cell((x, y)), snap.cell_mut((x, y))) {
                *dst = src.clone();
            }
        }
    }
    *app.popup_snapshot.borrow_mut() = Some((rect, snap));
}

/// Slide a freshly-opened popup down into place from off the top of the screen
/// over [`POPUP_ANIM`] (#35). Restores the dashboard backdrop where the popup
/// rests, then blits its snapshot shifted up by an easing offset (the whole
/// popup is above the top at the start), so it truly enters from off-screen.
fn render_popup_open(frame: &mut Frame, app: &App) {
    if !app.motion_enabled() || !crate::app::is_overlay_mode(app.mode) {
        return;
    }
    let now = std::time::Instant::now();
    let p = tween::progress(app.mode_entered_at, POPUP_ANIM, now);
    if p >= 1.0 {
        return;
    }
    let snap = app.popup_snapshot.borrow();
    let backdrop = app.popup_backdrop.borrow();
    let (Some((rect, buf)), Some(bd)) = (snap.as_ref(), backdrop.as_ref()) else {
        return;
    };
    // Off starts a full popup-height above the rest (fully off-screen) and eases
    // to 0. Restore the dashboard where the popup rests, then blit it shifted up.
    let off = ((1.0 - tween::ease_out(p)) * rect.bottom() as f32).round() as u16;
    let fb = frame.buffer_mut();
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            if let (Some(src), Some(dst)) = (bd.cell((x, y)), fb.cell_mut((x, y))) {
                *dst = src.clone();
            }
        }
    }
    for y in rect.top()..rect.bottom() {
        let Some(ty) = y.checked_sub(off) else {
            continue;
        };
        for x in rect.left()..rect.right() {
            if let (Some(src), Some(dst)) = (buf.cell((x, y)), fb.cell_mut((x, ty))) {
                *dst = src.clone();
            }
        }
    }
}

/// Blit a just-closed popup's captured snapshot, sliding it up off the top over
/// [`POPUP_ANIM`] (#35). The dashboard beneath is already drawn, so the popup
/// rises away revealing it.
fn render_popup_close(frame: &mut Frame, app: &App) {
    if !app.motion_enabled() {
        return;
    }
    let Some(at) = app.popup_closing_at else {
        return;
    };
    let now = std::time::Instant::now();
    let p = tween::progress(at, POPUP_ANIM, now);
    if p >= 1.0 {
        return;
    }
    let snap = app.popup_snapshot.borrow();
    let Some((rect, buf)) = snap.as_ref() else {
        return;
    };
    // Travel the popup's whole bottom edge to the top, so at p==1 every row has
    // slid above y==0 and nothing lingers near the top of the screen.
    let off = (tween::ease_out(p) * rect.bottom() as f32).round() as u16;
    let fb = frame.buffer_mut();
    for y in rect.top()..rect.bottom() {
        let Some(ty) = y.checked_sub(off) else {
            continue;
        };
        for x in rect.left()..rect.right() {
            if let (Some(src), Some(dst)) = (buf.cell((x, y)), fb.cell_mut((x, ty))) {
                *dst = src.clone();
            }
        }
    }
}

/// Slide the freshly-rendered full-screen session in from the right edge over
/// [`SESSION_ANIM`] (#35). Snapshots the session buffer, then blits it shifted
/// right by an easing offset, leaving the vacated left band blank so the view
/// reads as pushing in from the right.
fn render_session_enter(frame: &mut Frame, app: &App, composition: &FrameComposition) {
    // The captured offset, never a fresh clock reading: what this blit puts down
    // has to be exactly what `composition.protected` covers, or the ownership of
    // the leading columns is a millisecond — and therefore a column — off.
    let Some(off) = composition.enter_offset else {
        return;
    };
    let area = frame.area();
    // What the session is sliding over. Without it the columns it has not reached
    // yet come out blank, so entering a session flashed a black screen with the
    // host arriving over it.
    //
    // No usable dashboard behind it means no slide at all. A snapshot captured
    // under the previous theme is dropped by `invalidate_theme_visual_state`,
    // and one captured at a different terminal size holds cells for the wrong
    // geometry — blitting either (or the `Cell::reset()` the missing case used
    // to fall back to) puts foreign cells into the live session buffer. Showing
    // the session without its slide is the honest outcome.
    let behind = app.dashboard_snapshot.borrow();
    let Some(snapshot) = behind.as_ref().filter(|b| b.area == area) else {
        return;
    };
    let src = frame.buffer_mut().clone();
    let fb = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        // Right-to-left so each destination reads a not-yet-overwritten source.
        for x in (area.x..area.x + area.width).rev() {
            if let Some(sx) = x.checked_sub(off).filter(|sx| *sx >= area.x) {
                if let (Some(s), Some(d)) = (src.cell((sx, y)), fb.cell_mut((x, y))) {
                    *d = s.clone();
                }
            } else if let (Some(s), Some(d)) = (snapshot.cell((x, y)), fb.cell_mut((x, y))) {
                *d = s.clone();
            }
        }
    }
}

/// Slide a just-left session's captured snapshot off to the right over
/// [`SESSION_ANIM`] (#35), revealing the dashboard already drawn beneath. The
/// mirror of [`render_session_enter`].
fn render_session_exit(frame: &mut Frame, app: &App, composition: &FrameComposition) {
    let area = frame.area();
    // The captured offset, never a fresh clock reading: whatever this blit puts
    // down has to be exactly what `composition.protected` covers.
    let Some(off) = composition.exit_offset else {
        return;
    };
    let snap = app.session_snapshot.borrow();
    let Some(snapshot) = snap.as_ref() else {
        return;
    };
    if off >= area.width {
        return;
    }
    let fb = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        // Left-to-right: each destination x reads source x-off (already passed).
        for x in (area.x + off)..(area.x + area.width) {
            if let (Some(s), Some(d)) = (snapshot.buffer.cell((x - off, y)), fb.cell_mut((x, y))) {
                *d = s.clone();
            }
        }
    }
}

/// Slide between two embedded session tabs over [`TAB_ANIM`] (#35): the tab
/// being left is carried off one edge while the new one follows it in from the
/// other, so `Ctrl`+arrows reads as travel along the strip instead of a swap.
/// Shares the dashboard tab-switch duration, being the same gesture.
fn render_session_tab_slide(frame: &mut Frame, app: &App, composition: &FrameComposition) {
    let Some(sw) = app.session_tab_switch else {
        return;
    };
    let Some(e) = composition.tab_slide_progress else {
        return;
    };
    let snap = app.session_snapshot.borrow();
    let Some(outgoing) = snap.as_ref() else {
        return;
    };
    // Only the PTY body travels: the header stays put so the tab strip is a
    // fixed reference while its highlight slides between tabs (#35). The rect
    // is the viewport's own, so the region cleared and blitted here is exactly
    // the region the PTY renderer owns, on every terminal size.
    let area = crate::session::render::remote_pty_rect(frame.area());
    // The new tab is already drawn at rest; lift it so both layers can move.
    let incoming = blit::snapshot(frame.buffer_mut(), area);
    frame.render_widget(Clear, area);
    let w = area.width as f32;
    let dir = sw.dir as f32;
    let fb = frame.buffer_mut();
    blit::blit(
        fb,
        area,
        area,
        &outgoing.buffer,
        (-dir * e * w).round() as i32,
        0,
    );
    blit::blit(
        fb,
        area,
        area,
        &incoming,
        (dir * (1.0 - e) * w).round() as i32,
        0,
    );
}

/// How long the dashboard takes to fade up over the intro animation (#35).
pub const SPLASH_FADE: std::time::Duration = std::time::Duration::from_millis(360);

/// How long a panel's swapped-out content takes to fade in (#35).
pub const CONTENT_FADE: std::time::Duration = std::time::Duration::from_millis(140);

/// How long an SFTP pane's listing takes to slide to a new directory (#35).
pub const SFTP_NAV_ANIM: std::time::Duration = std::time::Duration::from_millis(200);

/// How long a newly staged SFTP transfer takes to fly into the queue (#35).
pub const SFTP_QUEUE_ANIM: std::time::Duration = std::time::Duration::from_millis(200);

/// How long a host's status dot flashes after its ping class changes (#35).
pub const PING_FLASH: std::time::Duration = std::time::Duration::from_millis(420);

/// Duration of a group's fold / unfold reveal in the host list (#35).
pub const FOLD_ANIM: std::time::Duration = std::time::Duration::from_millis(180);

/// Duration of the host-list highlight wipe under a moved cursor (#35).
pub const SELECT_ANIM: std::time::Duration = std::time::Duration::from_millis(120);

/// Duration of the tab-switch body slide (#35).
pub const TAB_ANIM: std::time::Duration = std::time::Duration::from_millis(220);

/// Duration of a popup's open / close slide (#35).
pub const POPUP_ANIM: std::time::Duration = std::time::Duration::from_millis(260);

/// Duration of the full-screen session-enter slide on connect (#35).
pub const SESSION_ANIM: std::time::Duration = std::time::Duration::from_millis(280);

/// Duration of an SFTP tab sub-state slide: picker <-> connecting <-> browser (#35).
pub const SFTP_ANIM: std::time::Duration = std::time::Duration::from_millis(260);

/// Shared popup rect hook (#35): every overlay runs its resting rect through
/// this so the render pass can snapshot the popup for its open/close slides.
/// Returns the rest rect unchanged — the popup always *draws* at rest, and the
/// slide is a separate blit pass ([`render_popup_open`] / [`render_popup_close`])
/// that can clip the popup above the top of the screen (a `Rect` cannot).
pub fn popup_open_rect(target: Rect, app: &App) -> Rect {
    app.last_popup_rect.set(Some(target));
    target
}

/// Dispatch a tab index to its body renderer, into `areas`.
fn render_tab_body(
    frame: &mut Frame,
    tab: usize,
    areas: &dashboard_layout::DashboardAreas,
    app: &App,
) {
    match tab {
        0 => render_hosts_body(frame, areas, app),
        1 => render_sftp_body(frame, areas, app),
        2 => render_tunnels_body(frame, areas, app),
        3 => render_keys_body(frame, areas, app),
        4 => render_audit_body(frame, areas, app),
        _ => render_hosts_body(frame, areas, app),
    }
}

/// Copy `areas` with the body region (body + the three columns) shifted right by
/// `dx` columns, for rendering a tab body mid-slide. Header/tab-bar/footer stay.
fn shift_body_areas(
    areas: &dashboard_layout::DashboardAreas,
    dx: u16,
) -> dashboard_layout::DashboardAreas {
    let shift = |r: Rect| Rect::new(r.x.saturating_add(dx), r.y, r.width, r.height);
    let mut a = *areas;
    a.body = shift(a.body);
    a.col_left = shift(a.col_left);
    a.col_mid = shift(a.col_mid);
    a.col_right = shift(a.col_right);
    a
}

/// Render a tab-switch slide: a static backdrop body plus the moving body
/// translated right by an eased offset, with a hard edge between them (#35).
/// `to > from` slides the new tab in from the right; `to < from` slides the old
/// tab out to the right, revealing the new one beneath.
fn render_tab_slide(
    frame: &mut Frame,
    areas: &dashboard_layout::DashboardAreas,
    app: &App,
    sw: crate::app::TabSwitch,
    now: std::time::Instant,
) {
    let p = tween::ease_out(tween::progress(sw.at, TAB_ANIM, now));
    let bw = areas.body.width;
    let right = sw.to > sw.from;
    // The moving layer sits on top starting at `body.x + off`; the backdrop shows
    // in `[body.x, body.x + off]`. Right: new enters from the right (off: bw->0).
    // Left: old exits to the right (off: 0->bw).
    let off = if right {
        ((1.0 - p) * bw as f32).round() as u16
    } else {
        (p * bw as f32).round() as u16
    };
    let (backdrop, top) = if right {
        (sw.from, sw.to)
    } else {
        (sw.to, sw.from)
    };

    render_tab_body(frame, backdrop, areas, app);
    if off < bw {
        let clear = Rect::new(
            areas.body.x + off,
            areas.body.y,
            bw - off,
            areas.body.height,
        );
        frame.render_widget(Clear, clear);
        let shifted = shift_body_areas(areas, off);
        render_tab_body(frame, top, &shifted, app);
    }
}

fn render_hosts_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    // Issue #18: a zoomed panel takes over the whole dashboard body.
    // Broadcast (#3) is a floating panel drawn from render_inner instead, so a
    // zoomed Broadcast must not be handled here (it has no home in the hosts
    // grid) — let render_inner's broadcast block own it.
    let grid_panel = app.focused_panel != crate::app::PanelId::Broadcast;
    let now = std::time::Instant::now();
    // A zoom morph (#35) is playing while the anim exists and hasn't finished.
    let morphing = grid_panel && app.zoom_anim.is_some_and(|a| !a.is_done(now));

    // Fully zoomed, no morph in flight: the panel owns the whole body.
    if app.panel_zoomed && !morphing && grid_panel {
        render_zoomed_panel(frame, areas.body, app);
        return;
    }
    widgets::hosts_panel::render_hosts_panel(frame, areas.col_left, app);
    widgets::middle_stack::render_middle_stack(frame, areas.col_mid, app);
    widgets::right_stack::render_right_stack(frame, areas.col_right, app);

    // SSH log panel spanning middle + right columns below their stacks
    let log_top = areas.col_mid.y + 19;
    let log_bottom = areas.footer.y.saturating_sub(2);
    if log_bottom > log_top + 3 {
        let log_area = Rect::new(
            areas.col_mid.x,
            log_top,
            areas.col_mid.width + 1 + areas.col_right.width,
            log_bottom - log_top,
        );
        widgets::middle_stack::render_ssh_log_panel(frame, log_area, app);
    }

    // Zoom morph (#35): overlay the focused panel at the interpolating rect over
    // the grid, so zoom-in grows out of the slot and zoom-out shrinks back into
    // it. When the morph finishes, the branch above takes over (full body) or
    // the plain grid remains.
    if morphing {
        if let Some(anim) = app.zoom_anim {
            let rect = anim.rect_at(now);
            frame.render_widget(Clear, rect);
            render_zoomed_panel(frame, rect, app);
        }
    }
}

/// The grid slot a panel morphs out of / back into for the zoom animation
/// (#35). Approximated by the panel's column (or the log strip), which reads
/// well without threading every sub-panel's exact rect out of the stacks.
pub fn panel_zoom_source(
    areas: &dashboard_layout::DashboardAreas,
    panel: crate::app::PanelId,
) -> Rect {
    use crate::app::PanelId;
    use widgets::middle_stack::{AGENT_H, HOST_H, LATENCY_H};
    use widgets::right_stack::{AUTH_H, PING_H, RECENT_H};
    let mid = areas.col_mid;
    let right = areas.col_right;
    // Each stacked panel's real slot (same heights the stacks lay out), so the
    // morph grows/shrinks in both dimensions from the actual box.
    match panel {
        PanelId::Hosts => areas.col_left,
        PanelId::Detail => Rect::new(mid.x, mid.y, mid.width, HOST_H),
        PanelId::Agent => Rect::new(mid.x, mid.y + HOST_H, mid.width, AGENT_H),
        PanelId::Latency => Rect::new(mid.x, mid.y + HOST_H + AGENT_H, mid.width, LATENCY_H),
        PanelId::Recent => Rect::new(right.x, right.y, right.width, RECENT_H),
        PanelId::Auth => Rect::new(right.x, right.y + RECENT_H, right.width, AUTH_H),
        PanelId::Ping => Rect::new(right.x, right.y + RECENT_H + AUTH_H, right.width, PING_H),
        // SSH log spans mid+right along the bottom (see render_hosts_body).
        PanelId::SshLog => Rect::new(
            mid.x,
            mid.y + 19,
            mid.width + 1 + right.width,
            areas.body.height.saturating_sub(19),
        ),
        // Broadcast morphs from its own docked rect, handled elsewhere.
        PanelId::Broadcast => screens::broadcast::docked_rect(areas.body),
    }
}

/// Render just the focused panel into `area` (the full dashboard body) for the
/// tmux-style zoom (issue #18).
fn render_zoomed_panel(frame: &mut Frame, area: Rect, app: &App) {
    use crate::app::PanelId;
    match app.focused_panel {
        PanelId::Hosts => widgets::hosts_panel::render_hosts_panel(frame, area, app),
        PanelId::Detail => widgets::middle_stack::render_host_panel(frame.buffer_mut(), area, app),
        PanelId::Agent => widgets::middle_stack::render_agent_panel(frame.buffer_mut(), area, app),
        PanelId::Latency => {
            widgets::middle_stack::render_latency_panel(frame.buffer_mut(), area, app)
        }
        PanelId::Recent => widgets::right_stack::render_recent_panel(frame.buffer_mut(), area, app),
        PanelId::Auth => widgets::right_stack::render_auth_panel(frame.buffer_mut(), area, app),
        PanelId::Ping => widgets::right_stack::render_ping_panel(frame.buffer_mut(), area, app),
        PanelId::SshLog => widgets::middle_stack::render_ssh_log_panel(frame, area, app),
        PanelId::Broadcast => screens::broadcast::render_broadcast_zoomed(frame, area, app),
    }
}

fn render_sftp_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    screens::sftp::render_sftp(frame, areas.body, app);
}

fn render_tunnels_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    screens::tunnels::render_tunnels(frame, areas.body, app);
}

fn render_keys_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    screens::keys::render_keys(frame, areas.body, app);
}

fn render_audit_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    screens::audit::render_audit(frame, areas.body, app);
}

#[derive(Clone, Copy)]
enum FormKind {
    Host,
    Identity,
    Keygen,
    Group,
}

/// Rows each form needs to show everything it draws, hint lines included.
const fn form_rows_needed(kind: FormKind) -> u16 {
    match kind {
        // 16 fields + a blank + 2 hint rows + 2 borders, plus the extra hint row
        // the password field adds.
        FormKind::Host => 23,
        FormKind::Identity => 14,
        FormKind::Keygen => 14,
        FormKind::Group => 12,
    }
}

fn render_form_popup(frame: &mut Frame, app: &App, kind: FormKind) {
    let area = frame.area();
    let popup_width = (area.width * 70 / 100).max(50).min(area.width);
    // Rows the form itself draws (fields + hint lines + both borders). 70% of a
    // 24-row terminal is 16, and the host form needs 22 — the overflow fell off
    // the bottom, taking the save/cancel and reveal hints with it, which is how
    // a masked password ended up looking unrevealable. `form_rows_needed` is
    // guarded by a render test at 80x24 so a new field cannot re-clip them.
    let popup_height = (area.height * 70 / 100)
        .max(form_rows_needed(kind))
        .min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

    let theme = app.theme();
    let border = popup_border_style(theme, popup_area);
    open_popup(frame, popup_area, theme);

    match kind {
        FormKind::Host => {
            if let Some(form) = app.host_form.as_ref() {
                frame.render_widget(
                    screens::host_form::render_host_form(
                        form,
                        &app.groups,
                        &app.identities,
                        &app.save_key_label(),
                        &app.config.keybinds.secret_field_hints(),
                        theme,
                        border,
                    ),
                    popup_area,
                );
            }
        }
        FormKind::Identity => {
            if let Some(form) = app.identity_form.as_ref() {
                frame.render_widget(
                    screens::keychain::render_identity_form(
                        form,
                        &app.save_key_label(),
                        &app.config.keybinds.secret_field_hints(),
                        theme,
                        border,
                    ),
                    popup_area,
                );
            }
        }
        FormKind::Keygen => {
            if let Some(form) = app.keygen_form.as_ref() {
                frame.render_widget(
                    screens::keygen::render_keygen_form(form, &app.save_key_label(), theme, border),
                    popup_area,
                );
            }
        }
        FormKind::Group => {
            if let Some(form) = app.group_form.as_ref() {
                let identity_name = form.default_identity_id.and_then(|id| {
                    app.identities
                        .iter()
                        .find(|i| i.id == id)
                        .map(|i| i.name.clone())
                });
                let parent_name = form.parent_id.and_then(|id| {
                    app.groups
                        .iter()
                        .find(|g| g.id == id)
                        .map(|g| g.name.clone())
                });
                frame.render_widget(
                    screens::group_form::render_group_form(
                        form,
                        identity_name.as_deref(),
                        parent_name.as_deref(),
                        theme,
                        border,
                    ),
                    popup_area,
                );
            }
        }
    }

    // Whichever form was rendered above drew its frame in `border`'s solid
    // fallback; this is the gradient half of that pair.
    paint_popup_border(frame, popup_area, theme);

    // Validation errors belong INSIDE the popup — the dashboard status bar is
    // hidden behind it, so a save failure otherwise looks like a stuck form.
    let notice = match kind {
        FormKind::Host => app.host_notice.as_deref(),
        FormKind::Identity => app.identity_notice.as_deref(),
        FormKind::Keygen => app.keygen_notice.as_deref(),
        FormKind::Group => app.group_notice.as_deref(),
    };
    if let Some(notice) = notice {
        let y = popup_area.y + popup_area.height.saturating_sub(2);
        if y > popup_area.y && popup_area.width > 4 {
            let msg = text::ellipsize(notice, popup_area.width as usize - 4);
            let error = app.theme().style(StyleRole::FormError);
            frame
                .buffer_mut()
                .set_string(popup_area.x + 2, y, &msg, error);
        }
    }
}

fn render_confirm_quit_popup(frame: &mut Frame, app: &App) {
    let active = app.tunnel_manager.active_count();
    let message = if active > 0 {
        format!("Quit sshub?\n{active} active tunnel(s) will be closed.")
    } else {
        "Quit sshub?".to_string()
    };
    let hint = "y: quit \u{2502} n: stay \u{2502} Esc: cancel";

    let area = frame.area();
    let popup_width = 44u16.min(area.width);
    let popup_height = if active > 0 { 6u16 } else { 5u16 }.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup_area = crate::tui::popup_open_rect(popup_area, app);
    // Quitting is reversible ("n: stay"), so the whole dialog is a warning.
    let theme = app.theme();
    let warning = theme.style(StyleRole::PopupWarning);

    open_popup(frame, popup_area, theme);
    frame.render_widget(
        Paragraph::new(format!("{message}\n{hint}"))
            .wrap(Wrap { trim: false })
            .style(warning)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled("Confirm quit", warning))
                    .border_style(warning),
            ),
        popup_area,
    );
}

fn render_confirm_discard_popup(frame: &mut Frame, app: &App) {
    let message = "Save changes?";
    let hint = "y: save \u{2502} n: discard \u{2502} Esc: back";

    let area = frame.area();
    let popup_width = 36u16.min(area.width);
    let popup_height = 5u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup_area = crate::tui::popup_open_rect(popup_area, app);
    // Nothing is lost either way — a warning, not an error.
    let theme = app.theme();
    let warning = theme.style(StyleRole::PopupWarning);

    open_popup(frame, popup_area, theme);
    frame.render_widget(
        Paragraph::new(format!("{message}\n{hint}"))
            .wrap(Wrap { trim: false })
            .style(warning)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled("Unsaved changes", warning))
                    .border_style(warning),
            ),
        popup_area,
    );
}

fn render_confirm_delete_popup(frame: &mut Frame, app: &App) {
    use crate::app::PendingDelete;
    let message = match &app.pending_delete {
        Some(PendingDelete::Host { name, .. }) => format!("Delete host '{name}'?"),
        Some(PendingDelete::Identity { name, .. }) => format!("Delete identity '{name}'?"),
        Some(PendingDelete::Group { name, .. }) => format!("Delete group '{name}'?"),
        Some(PendingDelete::Tunnel { label, .. }) => format!("Delete tunnel '{label}'?"),
        Some(PendingDelete::Snippet { name, .. }) => format!("Delete snippet '{name}'?"),
        Some(PendingDelete::SftpEntry { name, is_dir, .. }) => {
            if *is_dir {
                format!("Delete folder '{name}' and all its contents?")
            } else {
                format!("Delete '{name}'?")
            }
        }
        None => "Delete?".to_string(),
    };
    let area = frame.area();
    let popup_width = 54u16.min(area.width);
    // Wrap the message (a host name can be long) and size the box to fit.
    let inner_w = popup_width.saturating_sub(2).max(1) as usize;
    let msg_rows = message.chars().count().div_ceil(inner_w).max(1) as u16;
    let popup_height = (msg_rows + 4).min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let lines = vec![
        ratatui::text::Line::from(message),
        ratatui::text::Line::from(""),
        ratatui::text::Line::from("y: delete    Esc: cancel"),
    ];

    let popup_area = crate::tui::popup_open_rect(popup_area, app);
    // A delete cannot be undone: the frame is the error role, while the
    // question itself stays a warning — the two were red and yellow before.
    let theme = app.theme();
    let error = theme.style(StyleRole::PopupError);
    let warning = theme.style(StyleRole::PopupWarning);

    open_popup(frame, popup_area, theme);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(warning)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled("Confirm", error))
                    .border_style(error),
            ),
        popup_area,
    );
}

/// Format current UTC time as "Ddd HH:MM:SS".
fn format_utc_clock() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;

    // Day-of-week via Tomohiko Sakamoto's algorithm.
    // Convert unix timestamp to y/m/d then compute weekday.
    let days = (secs / 86400) as i64;
    // 1970-01-01 was a Thursday (weekday index 4).
    let weekday = ((days % 7 + 4) % 7) as usize;
    const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

    format!("{} {:02}:{:02}:{:02} UTC", DAY_NAMES[weekday], h, m, s)
}

/// Scroll ceiling for the help body given the full terminal area. Uses the same
/// popup geometry as `render_help_popup` (60% height, min 16; borders, query row,
/// and fixed footer), kept in one place so the key handler can't scroll past what
/// the renderer will show (the excess would be invisible "debt" that Up has to
/// unwind before the view moves).
pub(crate) fn help_max_scroll(area: Rect, query: &str) -> u16 {
    let popup_height = (area.height * 60 / 100).max(16).min(area.height);
    let body_height = popup_height.saturating_sub(4);
    screens::help::help_line_count(query).saturating_sub(body_height)
}

fn render_help_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let popup_width = (area.width * 70 / 100).max(40).min(area.width);
    let popup_height = (area.height * 60 / 100).max(16).min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup_area = crate::tui::popup_open_rect(popup_area, app);
    let theme = app.theme();

    open_popup(frame, popup_area, theme);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(popup_border_style(theme, popup_area))
            .title(Span::styled(" Help ", theme.style(StyleRole::PopupTitle))),
        popup_area,
    );
    paint_popup_border(frame, popup_area, theme);

    // Query + fixed footer; scroll only the body between them.
    let inner = popup_area.inner(Margin::new(1, 1));
    let query_line = format!("› {}\u{2588}", app.help_query);
    frame.buffer_mut().set_string(
        inner.x,
        inner.y,
        crate::tui::text::ellipsize(&query_line, inner.width as usize),
        theme.style(StyleRole::PickerQuery),
    );

    let body = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(2),
    );
    let scroll = app.help_scroll.min(help_max_scroll(area, &app.help_query));
    frame.render_widget(
        screens::help::render_help(scroll, &app.help_query, theme),
        body,
    );

    let footer_y = inner.y + inner.height.saturating_sub(1);
    frame.buffer_mut().set_string(
        inner.x,
        footer_y,
        crate::tui::text::ellipsize(screens::help::HELP_FOOTER, inner.width as usize),
        theme.style(StyleRole::PopupHint),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppDeps, HostEntry};
    use crate::config::AppConfig;
    use crate::metadata::{HostMetadata, MetadataDb};
    use crate::ssh::{HostResolver, SshHost};
    use crate::store::LauncherStore;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::sync::Arc;

    fn test_store() -> Arc<LauncherStore> {
        Arc::new(LauncherStore::open_in_memory().unwrap())
    }

    struct EmptyResolver;

    impl HostResolver for EmptyResolver {
        fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }

        fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
            Ok(SshHost::new(name))
        }
    }

    fn buffer_contains(buffer: &Buffer, needle: &str) -> bool {
        let area = buffer.area;
        for y in area.y..area.y + area.height {
            let line: String = (area.x..area.x + area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            if line.contains(needle) {
                return true;
            }
        }
        false
    }

    fn test_app_with_hosts() -> App {
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(EmptyResolver),
                metadata: Arc::new(MetadataDb::default()),
                store: test_store(),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        let mut web = SshHost::new("web-prod");
        web.hostname = Some("10.0.0.1".into());
        web.user = Some("ubuntu".into());
        web.port = Some(22);
        app.hosts = vec![HostEntry::Legacy {
            host: web,
            meta: HostMetadata {
                host_name: "web-prod".into(),
                tags: vec!["prod".into()],
                favorite: true,
                ..Default::default()
            },
        }];
        app.filtered_indices = vec![0];
        app.selected = 0;
        app.rebuild_filter();
        app
    }

    fn render_to_buffer(app: &App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    // ── Frozen default-theme golden ──────────────────────────
    //
    // The runtime theme system must not change how `default` looks. These
    // helpers freeze the current `theme.rs`-backed rendering cell by cell, so
    // any later migration that shifts a colour, a modifier or a glyph fails
    // loudly instead of quietly redecorating the app.

    /// Fixed clock text — the real one is wall-clock dependent.
    const GOLDEN_CLOCK: &str = "Mon 00:00:00";
    /// The scope path `render_inner` passes to the tab bar.
    const GOLDEN_SCOPE: &str = "~/.config/sshub";
    /// Columns at the right end of the tab-bar row holding the scope path and
    /// the build version. The version comes from `CARGO_PKG_VERSION` and moves
    /// with every release, so this tail is blanked before the signature is
    /// taken; the tab chrome left of it is what the golden guards.
    const GOLDEN_VOLATILE_TAIL: u16 = 48;

    fn signature_color(color: ratatui::style::Color) -> String {
        use ratatui::style::Color;
        match color {
            Color::Reset => "reset".to_string(),
            Color::Rgb(r, g, b) => format!("rgb:{r:02x}{g:02x}{b:02x}"),
            Color::Indexed(i) => format!("idx:{i}"),
            other => format!("{other:?}").to_lowercase(),
        }
    }

    /// One line per cell: `x,y,symbol,fg,bg,underline,modifiers`.
    fn buffer_signature(buffer: &Buffer) -> String {
        let area = buffer.area;
        let mut out = String::new();
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let cell = &buffer[(x, y)];
                out.push_str(&format!(
                    "{x},{y},{},{},{},{},{}\n",
                    cell.symbol(),
                    signature_color(cell.fg),
                    signature_color(cell.bg),
                    signature_color(cell.underline_color),
                    cell.modifier.bits(),
                ));
            }
        }
        out
    }

    fn assert_buffer_signature_matches(actual: &str, expected: &str) {
        fn cell_and_terminator(record: &str) -> (&str, &str) {
            if let Some(cell) = record.strip_suffix("\r\n") {
                (cell, "CRLF")
            } else if let Some(cell) = record.strip_suffix('\n') {
                (cell, "LF")
            } else {
                (record, "<end>")
            }
        }

        let actual_cells: Vec<_> = actual.split_inclusive('\n').collect();
        let expected_cells: Vec<_> = expected.split_inclusive('\n').collect();
        for (index, (actual_record, expected_record)) in
            actual_cells.iter().zip(expected_cells.iter()).enumerate()
        {
            let (actual_cell, actual_terminator) = cell_and_terminator(actual_record);
            let (expected_cell, expected_terminator) = cell_and_terminator(expected_record);
            if actual_cell != expected_cell {
                let coordinate = expected_cell
                    .split(',')
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(",");
                panic!(
                    "golden buffer first mismatch at {coordinate} (cell {index})\n\
                     expected: {expected_cell}\nactual:   {actual_cell}"
                );
            }
            if actual_terminator != expected_terminator {
                panic!(
                    "golden buffer terminator mismatch at cell {index}\n\
                     expected: {expected_terminator}\nactual:   {actual_terminator}"
                );
            }
        }
        if actual_cells.len() != expected_cells.len() {
            let index = actual_cells.len().min(expected_cells.len());
            let actual = actual_cells
                .get(index)
                .map(|record| cell_and_terminator(record).0)
                .unwrap_or("<missing>");
            let expected = expected_cells
                .get(index)
                .map(|record| cell_and_terminator(record).0)
                .unwrap_or("<missing>");
            panic!(
                "golden buffer length mismatch at cell {index}: expected {} cells, actual {}\n\
                 expected: {expected}\nactual:   {actual}",
                expected_cells.len(),
                actual_cells.len()
            );
        }
        if actual != expected {
            let byte = actual
                .bytes()
                .zip(expected.bytes())
                .position(|(actual, expected)| actual != expected)
                .unwrap_or_else(|| actual.len().min(expected.len()));
            panic!("golden buffer first unclassified byte mismatch at byte {byte}");
        }
    }

    #[test]
    fn golden_signature_failure_reports_only_the_first_different_cell() {
        let panic = std::panic::catch_unwind(|| {
            assert_buffer_signature_matches(
                "0,0,A,reset,reset,reset,0\n1,0,X,reset,reset,reset,0\n2,0,Z,reset,reset,reset,0\n",
                "0,0,A,reset,reset,reset,0\n1,0,B,reset,reset,reset,0\n2,0,Y,reset,reset,reset,0\n",
            );
        })
        .expect_err("different signatures must fail");
        let message = panic_message(panic);
        assert!(message.contains("first mismatch at 1,0"), "{message}");
        assert!(message.contains("expected: 1,0,B"), "{message}");
        assert!(message.contains("actual:   1,0,X"), "{message}");
        assert!(!message.contains("2,0,Y"), "{message}");
        assert!(!message.contains("2,0,Z"), "{message}");
    }

    #[test]
    fn golden_signature_failure_reports_a_missing_or_extra_cell() {
        let panic = std::panic::catch_unwind(|| {
            assert_buffer_signature_matches(
                "0,0,A,reset,reset,reset,0\n",
                "0,0,A,reset,reset,reset,0\n1,0,B,reset,reset,reset,0\n",
            );
        })
        .expect_err("different lengths must fail");
        let message = panic_message(panic);
        assert!(message.contains("length mismatch at cell 1"), "{message}");
        assert!(message.contains("expected: 1,0,B"), "{message}");
        assert!(message.contains("actual:   <missing>"), "{message}");
    }

    #[test]
    fn golden_signature_failure_reports_a_missing_final_line_feed() {
        let panic = std::panic::catch_unwind(|| {
            assert_buffer_signature_matches(
                "0,0,A,reset,reset,reset,0",
                "0,0,A,reset,reset,reset,0\n",
            );
        })
        .expect_err("the serialized signatures differ byte-for-byte");
        let message = panic_message(panic);
        assert!(
            message.contains("terminator mismatch at cell 0"),
            "{message}"
        );
        assert!(message.contains("expected: LF"), "{message}");
        assert!(message.contains("actual:   <end>"), "{message}");
    }

    #[test]
    fn golden_signature_failure_distinguishes_crlf_from_lf() {
        let panic = std::panic::catch_unwind(|| {
            assert_buffer_signature_matches(
                "0,0,A,reset,reset,reset,0\r\n",
                "0,0,A,reset,reset,reset,0\n",
            );
        })
        .expect_err("CRLF and LF differ byte-for-byte");
        let message = panic_message(panic);
        assert!(
            message.contains("terminator mismatch at cell 0"),
            "{message}"
        );
        assert!(message.contains("expected: LF"), "{message}");
        assert!(message.contains("actual:   CRLF"), "{message}");
    }

    fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                panic
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_string())
            })
            .unwrap_or_else(|| "non-string panic".to_string())
    }

    /// The dashboard chrome every tab shares — header, session strip, tab bar,
    /// hosts body, separators and footer — rendered without the overlay,
    /// animation and popup paths that carry their own timing.
    fn render_default_theme_golden_surface(app: &App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                let areas = dashboard_layout::dashboard_layout_zoomed(area, app.ui_zoom);
                // Through `app.theme()`, not the legacy constants: that is what
                // turns the frozen buffer from a claim about `theme.rs` into an
                // end-to-end guarantee about the `default` theme's roles.
                let theme = app.theme();

                let [total, online, slow, down] = compute_header_stats(app);
                widgets::header::render_header(
                    frame,
                    areas.header,
                    widgets::header::HeaderStats {
                        host_count: total,
                        online,
                        slow,
                        down,
                        clock: GOLDEN_CLOCK,
                    },
                    theme,
                );
                let chips = build_session_chips(app);
                widgets::header::render_session_strip(frame, areas.header, &chips, None, theme);

                let rule1 = row_in(area, areas.header.y + areas.header.height);
                widgets::footer::render_hrule(
                    frame,
                    rule1,
                    false,
                    theme,
                    PaintRole::HeaderSeparator,
                );
                widgets::tab_bar::render_tab_bar(
                    frame,
                    areas.tab_bar,
                    app.active_tab + 1,
                    GOLDEN_SCOPE,
                    theme,
                );
                let rule2 = row_in(area, areas.tab_bar.y + areas.tab_bar.height);
                widgets::footer::render_hrule(frame, rule2, false, theme, PaintRole::TabsSeparator);

                render_tab_body(frame, app.active_tab, &areas, app);

                let rule3 = row_in(area, areas.footer.y.saturating_sub(1));
                widgets::footer::render_hrule(
                    frame,
                    rule3,
                    true,
                    theme,
                    PaintRole::FooterSeparator,
                );
                let (keybinds, pinned) = footer_keybinds(app);
                widgets::footer::render_footer(frame, areas.footer, &keybinds, pinned, theme);
            })
            .unwrap();

        let mut buffer = terminal.backend().buffer().clone();
        let tab_bar = dashboard_layout::dashboard_layout_zoomed(buffer.area, app.ui_zoom).tab_bar;
        let from = tab_bar.right().saturating_sub(GOLDEN_VOLATILE_TAIL);
        for x in from..tab_bar.right() {
            if let Some(cell) = buffer.cell_mut((x, tab_bar.y)) {
                cell.reset();
            }
        }
        buffer
    }

    /// App with three sessions in distinct phases plus an open picker.
    fn app_with_picker(purpose: crate::app::SessionPickerPurpose, query: &str) -> App {
        use crate::session::{SessionConfig, SessionMeta, SessionPhase};
        use std::time::Instant;

        let mut app = test_app_with_hosts();
        for (name, user, addr) in [
            ("web-prod", "micha", "10.0.0.11"),
            ("dev-box", "deploy", "10.0.0.12"),
            ("db-old", "root", "10.0.0.13"),
        ] {
            let cfg = SessionConfig {
                argv: vec!["true".into()],
                display_name: name.into(),
                meta: SessionMeta {
                    user: Some(user.into()),
                    address: Some(addr.into()),
                    port: Some(22),
                    ..Default::default()
                },
                pending_secret: None,
                key_push_identity: None,
                host_name: name.into(),
            };
            app.sessions
                .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
        }
        app.sessions[1].phase = SessionPhase::Running {
            started_at: Instant::now(),
        };
        app.sessions[2].phase = SessionPhase::Exited {
            status: "exit 1".into(),
            at: Instant::now(),
        };
        app.active_session = Some(1);
        app.session_picker = Some(crate::app::SessionPicker {
            purpose,
            query: query.into(),
            selected: 0,
            return_mode: AppMode::Normal,
        });
        app.mode = AppMode::SessionPicker;
        app
    }

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    /// Read `n` cells of row `y` starting at column `x`.
    fn cells_at(buf: &ratatui::buffer::Buffer, x: u16, y: u16, n: u16) -> String {
        (x..(x + n).min(buf.area.right()))
            .map(|i| buf[(i, y)].symbol())
            .collect()
    }

    /// Column and row of the picker line carrying lifecycle word `word`.
    ///
    /// Matching the dot *plus* the padded word is what makes this unambiguous:
    /// the dashboard behind the popup draws its own session chips as
    /// `● <name>`, and a bare `find("up")` would also hit "backup" or "groups".
    fn picker_row(buf: &ratatui::buffer::Buffer, word: &str) -> (u16, u16) {
        let needle = format!("\u{25cf} {word:<4} ");
        let n = needle.chars().count() as u16;
        for y in buf.area.y..buf.area.bottom() {
            for x in buf.area.x..buf.area.right() {
                if cells_at(buf, x, y, n) == needle {
                    return (x, y);
                }
            }
        }
        panic!("no picker row for {word:?}");
    }

    /// First column of `needle` anywhere in `buf`, searched row by row.
    fn find_cell(buf: &Buffer, needle: &str) -> Option<(u16, u16)> {
        let n = needle.chars().count() as u16;
        for y in buf.area.y..buf.area.bottom() {
            for x in buf.area.x..buf.area.right().saturating_sub(n) {
                let got: String = (x..x + n).map(|i| buf[(i, y)].symbol()).collect();
                if got == needle {
                    return Some((x, y));
                }
            }
        }
        None
    }

    /// App with two spawned sessions, for the dashboard session strip.
    fn app_with_two_sessions() -> App {
        let mut app = test_app_with_hosts();
        for name in ["alpha", "bravo"] {
            let cfg = crate::session::SessionConfig {
                argv: vec!["true".into()],
                display_name: name.into(),
                meta: crate::session::SessionMeta::default(),
                pending_secret: None,
                key_push_identity: None,
                host_name: name.into(),
            };
            app.sessions
                .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
        }
        app
    }

    #[test]
    fn agent_panel_does_not_overprint_a_half_scrolled_card_row() {
        // The overlap is only reachable with a particular geometry, worked out
        // from the real numbers rather than guessed: the body must have at least
        // three spare rows after whole card rows (`height % 7 >= 3`), so the panel
        // is drawn at all, and the scroll must lag its goal by two lines or more,
        // so the cards sit lower than the whole-row arithmetic assumes.
        //
        // A 40-row terminal gives a 32-row body: four card rows of stride 7 fit
        // with four spare. Twelve identities in two columns make six rows; the
        // selection on row 4 puts the goal at line 14, and a scroll still at line
        // 10 pushes the last drawn card down to rows 31..36 -- straight through the
        // panel the old placement put at 34.
        let mut app = test_app_with_hosts();
        app.active_tab = 3;
        app.config.appearance.identity_columns = 2;
        app.identities = (0..12)
            .map(|i| crate::store::Identity {
                id: i as i64 + 1,
                name: format!("key-{i}"),
                username: Some("root".into()),
                private_key: Some(format!("/home/me/.ssh/sshub_key_{i}").into()),
                certificate: None,
                has_password: true,
            })
            .collect();
        app.identity_selected = 8;
        app.agent_info = Some(crate::ssh::agent::AgentInfo {
            socket_path: None,
            keys: Vec::new(),
            forwarding_hosts: 0,
        });
        app.keys_scroll_pos.set(10.0);
        app.keys_scroll_at.set(Some(std::time::Instant::now()));

        let buffer = render_to_buffer(&app, 120, 40);
        let (_, ly) = find_cell(&buffer, "loaded keys").expect("agent panel drawn");

        // The grid shows whole card rows only: a card cut by the grid's bottom
        // left a sliver above the rule that slid around while the rest sat still.
        // So no card border may appear on the rule's row or the one above it.
        let rule = ly - 1;
        for row in [rule - 1, rule] {
            let text: String = (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, row)].symbol())
                .collect();
            assert!(
                !text.contains('\u{250c}') && !text.contains('\u{2510}'),
                "row {row} carries a card's top border: {:?}",
                text.trim_end()
            );
        }

        // Both text rows of the panel must be the panel's alone. Card borders and
        // key paths bleeding in is what this looked like on screen:
        //   agent socket  (not set)────────────┘  └──────────┘
        for row in [ly - 1, ly] {
            let text: String = (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, row)].symbol())
                .collect();
            for leftover in [
                "\u{2518}",
                "\u{2514}",
                "\u{2502}",
                ".ssh/sshub_key",
                "passphrase",
                "not loaded",
            ] {
                assert!(
                    !text.contains(leftover),
                    "row {row} carries {leftover:?} from a card: {:?}",
                    text.trim_end()
                );
            }
        }
    }

    #[test]
    fn cycling_tabs_from_the_dashboard_stays_on_the_dashboard() {
        use crate::config::KeyAction;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = app_with_two_sessions();
        app.active_session = Some(0);
        app.mode = AppMode::Normal;
        app.config
            .keybinds
            .set(KeyAction::SessionTabNext, vec!["F6".into()]);

        app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::empty()))
            .unwrap();

        // The regression: this used to call `focus_active_session`, so a key
        // named "next session tab" threw you into the session full screen.
        assert_eq!(
            app.mode,
            AppMode::Normal,
            "cycling must not enter a session"
        );
        assert_eq!(app.active_session, Some(1), "the selection moved");
        assert!(
            app.session_tab_switch.is_some(),
            "the travel is armed from the dashboard too"
        );
    }

    #[test]
    fn the_session_slides_in_over_the_dashboard_not_over_black() {
        let mut app = app_with_two_sessions();
        app.active_session = Some(0);
        app.mode = AppMode::Normal;

        // Rendering the dashboard is what captures the snapshot the slide needs.
        let dashboard = render_to_buffer(&app, 120, 38);
        let (hx, hy) = find_cell(&dashboard, "web-prod").expect("dashboard drawn");

        // First frame of the slide: the session is still fully off to the right,
        // so what shows is the dashboard. It used to be blank cells, which read as
        // a black screen flashing before the host arrived.
        app.mode = AppMode::Session;
        app.session_enter_at = Some(std::time::Instant::now());
        let sliding = render_to_buffer(&app, 120, 38);
        assert_eq!(
            sliding[(hx, hy)].symbol(),
            dashboard[(hx, hy)].symbol(),
            "the vacated columns show the dashboard"
        );
        assert_ne!(sliding[(hx, hy)].symbol(), " ", "and are not blanked");
    }

    /// Drive `render_session_enter` alone over a recognisable buffer and report
    /// what it left behind, so a blit can be told from a no-op cell by cell.
    fn session_enter_over_pattern(app: &App, width: u16, height: u16) -> (Buffer, Buffer) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut prepared = None;
        terminal
            .draw(|frame| {
                let area = frame.area();
                let buf = frame.buffer_mut();
                for y in area.y..area.bottom() {
                    for x in area.x..area.right() {
                        buf[(x, y)].set_symbol(if (x + y) % 2 == 0 { "#" } else { "." });
                    }
                }
                prepared = Some(buf.clone());
                let composition = FrameComposition::capture(app, area, std::time::Instant::now());
                render_session_enter(frame, app, &composition);
            })
            .unwrap();
        (prepared.unwrap(), terminal.backend().buffer().clone())
    }

    /// A session sliding in with no usable dashboard behind it must draw
    /// nothing at all.
    ///
    /// The vacated columns used to fall back to `Cell::reset()`, so a missing
    /// snapshot blitted hard resets — and a snapshot captured at a different
    /// terminal size blitted cells from the wrong geometry — into the live
    /// session buffer. Both are stale-theme leaks, not merely cosmetic.
    #[test]
    fn a_session_slide_without_a_fresh_dashboard_snapshot_draws_nothing() {
        let mut app = app_with_two_sessions();
        app.active_session = Some(0);
        app.mode = AppMode::Session;
        app.session_enter_at = Some(std::time::Instant::now());

        // No snapshot at all.
        *app.dashboard_snapshot.borrow_mut() = None;
        let (prepared, after) = session_enter_over_pattern(&app, 40, 10);
        assert_eq!(after, prepared, "a missing snapshot must be a no-op");

        // A snapshot from a differently sized terminal is just as unusable.
        *app.dashboard_snapshot.borrow_mut() = Some(Buffer::empty(Rect::new(0, 0, 20, 6)));
        let (prepared, after) = session_enter_over_pattern(&app, 40, 10);
        assert_eq!(after, prepared, "a stale-area snapshot must be a no-op");

        // The control: a snapshot matching the render area still slides.
        *app.dashboard_snapshot.borrow_mut() = Some(Buffer::empty(Rect::new(0, 0, 40, 10)));
        let (prepared, after) = session_enter_over_pattern(&app, 40, 10);
        assert_ne!(after, prepared, "a fresh snapshot must still animate");
    }

    #[test]
    fn entering_a_session_from_the_dashboard_slides_it_in() {
        use crate::config::KeyAction;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = app_with_two_sessions();
        app.active_session = Some(0);
        app.mode = AppMode::Normal;
        app.config
            .keybinds
            .set(KeyAction::SessionFocus, vec!["F7".into()]);

        app.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::empty()))
            .unwrap();
        assert!(
            crate::app::is_session_mode(app.mode),
            "we are in the session"
        );
        assert!(
            app.session_enter_at.is_some(),
            "arriving animates, the same way leaving already did"
        );

        // Re-deriving the mode while already inside a session is not an entry
        // and must not replay the slide.
        app.session_enter_at = None;
        app.focus_active_session();
        assert!(app.session_enter_at.is_none());

        // Reduced motion arms nothing.
        app.mode = AppMode::Normal;
        app.config.appearance.disable_animation = true;
        app.focus_active_session();
        assert!(crate::app::is_session_mode(app.mode));
        assert!(app.session_enter_at.is_none());
    }

    #[test]
    fn dashboard_strip_highlight_travels_instead_of_teleporting() {
        let mut app = app_with_two_sessions();
        let active_bg = app
            .theme()
            .style(StyleRole::HeaderSessionActive)
            .bg
            .expect("the active session chip has a background");

        // At rest the highlight sits on the active chip, as before.
        app.active_session = Some(1);
        let buffer = render_to_buffer(&app, 120, 38);
        let (bx, by) = find_cell(&buffer, "bravo").expect("second chip rendered");
        assert_eq!(buffer[(bx, by)].bg, active_bg, "at rest: on the new chip");

        // Mid-switch, with progress still at ~0, the highlight must still be on
        // the chip being left. That is the whole point: it moves across rather
        // than appearing on the target instantly.
        app.session_tab_switch = Some(crate::app::SessionTabSwitch {
            dir: 1,
            from: 0,
            at: std::time::Instant::now(),
        });
        let buffer = render_to_buffer(&app, 120, 38);
        let (ax, ay) = find_cell(&buffer, "alpha").expect("first chip rendered");
        let (bx, by) = find_cell(&buffer, "bravo").expect("second chip rendered");
        assert_eq!(
            buffer[(ax, ay)].bg,
            active_bg,
            "travelling: still on the chip being left"
        );
        assert_ne!(
            buffer[(bx, by)].bg,
            active_bg,
            "travelling: not yet on the target"
        );

        // Reduced motion jumps straight to the final state.
        app.config.appearance.disable_animation = true;
        let buffer = render_to_buffer(&app, 120, 38);
        let (ax, ay) = find_cell(&buffer, "alpha").unwrap();
        let (bx, by) = find_cell(&buffer, "bravo").unwrap();
        assert_eq!(buffer[(bx, by)].bg, active_bg, "reduced motion: target");
        assert_ne!(buffer[(ax, ay)].bg, active_bg);
    }

    #[test]
    fn sftp_footer_points_back_to_local_once_the_left_pane_is_remote() {
        let mut app = test_app_with_hosts();
        app.active_tab = 1;
        app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));

        // 120 columns on purpose: the SFTP row does not fit there, so this also
        // pins that the pair survives the truncation rather than only existing.
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "2nd host"));
        assert!(!buffer_contains(&buffer, "O local"));

        // Pointed at a second server, the footer has to say how to get back;
        // `o` only leads further away and nothing else on screen mentions `O`.
        app.sftp.as_mut().unwrap().left_host = Some("bravo".into());
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "O local"));
        assert!(!buffer_contains(&buffer, "2nd host"));
    }

    #[test]
    fn narrow_footer_keeps_help_and_quit_and_marks_the_gap() {
        // The SFTP tab has the longest row: 220 columns to show all of it.
        let mut app = test_app_with_hosts();
        app.active_tab = 1;
        app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));

        for w in [80u16, 100, 120, 160, 200] {
            let buffer = render_to_buffer(&app, w, 38);
            assert!(buffer_contains(&buffer, "? help"), "width {w}: help");
            assert!(buffer_contains(&buffer, "q quit"), "width {w}: quit");
            assert!(
                buffer_contains(&buffer, "\u{2026}"),
                "width {w}: dropped pairs are marked"
            );
        }

        // With sessions running, the way back into one is as essential as the
        // way out of the app. This is the case that regressed: the session hints
        // are appended after `? help` / `q quit` and pushed them into the middle.
        let mut app = app_with_two_sessions();
        app.active_tab = 1;
        app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));
        for w in [120u16, 160, 200] {
            let buffer = render_to_buffer(&app, w, 38);
            assert!(buffer_contains(&buffer, "resume"), "width {w}: resume");
            assert!(buffer_contains(&buffer, "? help"), "width {w}: help");
            assert!(buffer_contains(&buffer, "q quit"), "width {w}: quit");
        }

        // Wide enough for everything: no ellipsis, nothing dropped.
        let mut app = test_app_with_hosts();
        app.active_tab = 1;
        app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));
        let buffer = render_to_buffer(&app, 240, 38);
        assert!(buffer_contains(&buffer, "? help"));
        assert!(buffer_contains(&buffer, "q quit"));
        assert!(buffer_contains(&buffer, "/ search"));
        assert!(!buffer_contains(&buffer, "\u{2026}"));
    }

    #[test]
    fn default_dashboard_matches_frozen_legacy_buffer() {
        let buffer = render_default_theme_golden_surface(&test_app_with_hosts(), 132, 38);
        assert_buffer_signature_matches(
            &buffer_signature(&buffer),
            include_str!("../../tests/fixtures/theme/default-dashboard.buffer"),
        );
    }

    // ── Theme-driven app background ─────────────────────────
    //
    // Three background states (terminal / explicit solid / explicit gradient)
    // crossed with the two transparency switches, plus the rule that decides
    // the whole task: SSHub's own ground never reaches the remote PTY viewport,
    // which has a ground — and a switch — of its own.

    /// An app whose live theme is the user file `body`, kept alive by the
    /// returned temp dir. Never touches the real HOME or config.
    fn app_with_user_theme(id: &str, body: &str) -> (App, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let themes = root.path().join("themes");
        std::fs::create_dir(&themes).unwrap();
        std::fs::write(themes.join(format!("{id}.toml")), body).unwrap();
        let mut app = test_app_with_hosts();
        app.config.appearance.active_theme = id.to_string();
        app.load_themes_from(&themes);
        assert_eq!(app.theme().id().as_str(), id, "`{id}` must be live");
        (app, root)
    }

    /// A theme whose app background is a black-to-white horizontal sweep, so a
    /// per-cell sample is distinguishable from a flattened one.
    const GRADIENT_BACKGROUND_THEME: &str = "schema_version = 1\nname = \"Washed\"\n\
         extends = \"default\"\n\n\
         [gradients.wash]\ndirection = \"horizontal\"\n\
         stops = [ { at = 0.0, color = \"#000000\" }, { at = 1.0, color = \"#ffffff\" } ]\n\n\
         [components.app]\nbackground = { gradient = \"gradients.wash\" }\n";

    /// A dashboard app on the built-in `theme_id`.
    fn app_with_builtin_theme(theme_id: &str) -> App {
        let mut app = test_app_with_hosts();
        assert!(app.activate_theme(theme_id), "`{theme_id}` is a built-in");
        app
    }

    /// A full-screen session app on `theme_id`, with a live PTY grid in the
    /// body (phase `Running`, so `render_body` draws the tui-term widget).
    fn session_app_with_theme(theme_id: &str) -> App {
        let mut app = app_with_builtin_theme(theme_id);
        enter_live_session(&mut app);
        app
    }

    /// The colour the fake remote shell writes its marker line in — a value no
    /// SSHub theme uses, so a cell carrying it can only have come from the PTY.
    const REMOTE_FG: ratatui::style::Color = ratatui::style::Color::Rgb(0x12, 0x34, 0x56);

    /// A remote cell with an **RGB foreground and a `Reset` background**: the
    /// only cell shape that can tell the two failure modes apart. A theme pass
    /// that recolours the PTY shows up as a changed `bg`; a fade that runs over
    /// it shows up as a changed `fg`.
    fn write_remote_marker(app: &mut App) {
        app.sessions[0].parser.process(b"\x1b[38;2;18;52;86mREMOTE");
    }

    /// Where the marker landed, and what it must still look like.
    fn remote_marker_cell(area: Rect) -> (u16, u16) {
        let pty = crate::session::render::remote_pty_rect(area);
        (pty.x, pty.y)
    }

    /// Column of the marker on its row, found by glyph — the one property no
    /// fade or background pass alters, so the search still works in exactly the
    /// frames where a colour has gone wrong.
    fn find_remote_marker(buffer: &Buffer, row: u16) -> u16 {
        (buffer.area.x..buffer.area.right())
            .find(|x| buffer[(*x, row)].symbol() == "R")
            .unwrap_or_else(|| panic!("no remote marker on row {row}"))
    }

    fn assert_remote_cell_untouched_by_theme(
        buffer: &Buffer,
        at: (u16, u16),
        expected_bg: ratatui::style::Color,
        what: &str,
    ) {
        let cell = &buffer[at];
        assert_eq!(cell.symbol(), "R", "{what}: the marker is not at {at:?}");
        assert_eq!(
            cell.fg, REMOTE_FG,
            "{what}: the remote foreground was blended at {at:?}"
        );
        assert_eq!(
            cell.bg, expected_bg,
            "{what}: the remote background was repainted at {at:?}"
        );
    }

    /// Put `app` into the full-screen session view over a live PTY grid.
    fn enter_live_session(app: &mut App) {
        use crate::session::{SessionConfig, SessionMeta, SessionPhase};

        let cfg = SessionConfig {
            argv: vec!["true".into()],
            display_name: "web-prod".into(),
            meta: SessionMeta {
                user: Some("micha".into()),
                address: Some("10.0.0.11".into()),
                port: Some(22),
                ..Default::default()
            },
            pending_secret: None,
            key_push_identity: None,
            host_name: "web-prod".into(),
        };
        let mut session = crate::session::Session::spawn(cfg, 24, 80, None).unwrap();
        session.phase = SessionPhase::Running {
            started_at: std::time::Instant::now(),
        };
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Session;
    }

    /// A coordinate inside the remote PTY viewport that the shell never wrote,
    /// taken from the same geometry function the renderer uses.
    fn remote_pty_probe(area: Rect) -> (u16, u16) {
        let pty = crate::session::render::remote_pty_rect(area);
        assert!(pty.height >= 2 && pty.width >= 2, "no PTY body at {area:?}");
        // Two rows down: the first row may carry whatever `true` printed.
        (pty.x + pty.width / 2, pty.y + pty.height / 2)
    }

    /// A cell that is genuinely app background: the separator row under the
    /// header only ever sets a foreground, so its background is whatever the
    /// app ground pass left there.
    fn app_background_probe(buffer: &Buffer) -> (u16, u16) {
        let areas = dashboard_layout::dashboard_layout_zoomed(buffer.area, 0);
        (
            buffer.area.right() - 1,
            areas.header.y + areas.header.height,
        )
    }

    fn any_background_is_reset(buffer: &Buffer) -> bool {
        let a = buffer.area;
        (a.y..a.bottom())
            .any(|y| (a.x..a.right()).any(|x| buffer[(x, y)].bg == ratatui::style::Color::Reset))
    }

    fn assert_all_backgrounds_non_reset(buffer: &Buffer) {
        let a = buffer.area;
        for y in a.y..a.bottom() {
            for x in a.x..a.right() {
                assert_ne!(
                    buffer[(x, y)].bg,
                    ratatui::style::Color::Reset,
                    "cell ({x}, {y}) stayed transparent"
                );
            }
        }
    }

    #[test]
    fn theme_background_terminal_leaves_the_canvas_transparent() {
        // `default` resolves `components.app.background` to "terminal", so no
        // theme painting happens at all. The canvas fill would still back it,
        // which is what asking for transparency switches off.
        let mut app = app_with_builtin_theme("default");
        app.config.appearance.transparent_sshub_background = true;
        assert!(any_background_is_reset(&render_to_buffer(&app, 80, 24)));
    }

    #[test]
    fn theme_background_explicit_solid_paints_the_whole_dashboard() {
        let mut app = app_with_builtin_theme("aqua");
        app.config.appearance.transparent_sshub_background = false;
        let buffer = render_to_buffer(&app, 80, 24);
        assert_all_backgrounds_non_reset(&buffer);
        let expected = match app.theme().paint(PaintRole::AppBackground) {
            ResolvedPaint::Solid(color) => *color,
            other => panic!("aqua's app background is solid, got {other:?}"),
        };
        assert_eq!(buffer[app_background_probe(&buffer)].bg, expected);
    }

    #[test]
    fn theme_background_explicit_gradient_is_sampled_per_cell() {
        let (mut app, _dir) = app_with_user_theme("washed", GRADIENT_BACKGROUND_THEME);
        app.config.appearance.transparent_sshub_background = false;
        let buffer = render_to_buffer(&app, 80, 24);
        // The footer row is chrome SSHub leaves transparent, so the whole row
        // is app background — and it must sweep rather than sit on one colour.
        let y = buffer.area.bottom() - 1;
        let row: Vec<_> = (0..buffer.area.width).map(|x| buffer[(x, y)].bg).collect();
        assert!(
            row.windows(2).any(|pair| pair[0] != pair[1]),
            "the app background is flat: {row:?}"
        );
    }

    #[test]
    fn the_canvas_backs_a_theme_that_claims_no_ground() {
        // Opaque out of the box: `default` paints nothing itself, so the canvas
        // is what keeps SSHub readable without the user asking for anything.
        let app = app_with_builtin_theme("default");
        let buffer = render_to_buffer(&app, 80, 24);
        assert_all_backgrounds_non_reset(&buffer);
        assert_eq!(
            buffer[app_background_probe(&buffer)].bg,
            app.theme().semantic().canvas
        );
    }

    #[test]
    fn the_canvas_fill_cannot_override_an_explicit_surface() {
        // The canvas fills what is *left*; a theme that painted the app surface
        // keeps it (spec: it "kann eine ausdrücklich gesetzte App-Fläche nicht
        // unterdrücken").
        // `fire` is the built-in whose canvas differs from its background, so
        // the two fills are distinguishable cell by cell.
        let app = app_with_builtin_theme("fire");
        let buffer = render_to_buffer(&app, 80, 24);
        let expected = match app.theme().paint(PaintRole::AppBackground) {
            ResolvedPaint::Solid(color) => *color,
            other => panic!("fire's app background is solid, got {other:?}"),
        };
        assert_ne!(
            expected,
            app.theme().semantic().canvas,
            "this test needs a theme whose canvas and background differ"
        );
        assert_eq!(buffer[app_background_probe(&buffer)].bg, expected);
    }

    #[test]
    fn the_app_surface_never_reaches_the_remote_pty() {
        // `aqua` sweeps a gradient across `components.app.background`. The grid
        // gets the theme's flat PTY ground instead — a gradient under arbitrary
        // remote output has no stable contrast against the remote's own colours.
        let mut app = session_app_with_theme("aqua");
        app.config.appearance.transparent_sshub_background = false;
        let buffer = render_to_buffer(&app, 80, 24);
        // The session's own chrome rows are SSHub-owned and get painted.
        assert_ne!(buffer[(1, 0)].bg, ratatui::style::Color::Reset);

        let pty = crate::session::render::remote_pty_rect(buffer.area);
        let row: Vec<_> = (pty.x..pty.right())
            .map(|x| buffer[(x, pty.y + 1)].bg)
            .collect();
        assert!(
            row.windows(2).all(|pair| pair[0] == pair[1]),
            "a gradient reached the remote PTY: {row:?}"
        );
        assert_eq!(
            row[0],
            app.theme().semantic().pty_background,
            "the grid must carry the theme's PTY ground, flat"
        );
    }

    #[test]
    fn a_gradient_background_never_reaches_the_remote_pty() {
        let (mut app, _dir) = app_with_user_theme("washed", GRADIENT_BACKGROUND_THEME);
        // Release the grid, so a gradient that did reach it would show up as a
        // painted cell instead of hiding behind the grid's own ground.
        app.config.appearance.transparent_session_background = true;
        enter_live_session(&mut app);
        let buffer = render_to_buffer(&app, 80, 24);
        // The session chrome above the grid is SSHub's, and the sweep reaches it.
        let header: Vec<_> = (0..buffer.area.width).map(|x| buffer[(x, 0)].bg).collect();
        assert!(
            header.windows(2).any(|pair| pair[0] != pair[1]),
            "the gradient never reached the session chrome: {header:?}"
        );
        let probe = remote_pty_probe(buffer.area);
        assert_eq!(buffer[probe].bg, ratatui::style::Color::Reset);
    }

    /// The two explicit-background themes the matrix is run against: a solid
    /// whose canvas differs from its background, and a gradient.
    fn explicit_background_themes() -> Vec<(&'static str, Option<tempfile::TempDir>)> {
        vec![("fire", None), ("washed", None)]
    }

    /// An app on `theme_id`, where `"washed"` is the gradient user theme.
    fn app_on(theme_id: &str) -> (App, Option<tempfile::TempDir>) {
        if theme_id == "washed" {
            let (app, dir) = app_with_user_theme("washed", GRADIENT_BACKGROUND_THEME);
            (app, Some(dir))
        } else {
            (app_with_builtin_theme(theme_id), None)
        }
    }

    /// What the PTY's background may legitimately be: transparent once
    /// `transparent_session_background` released it, the theme's own PTY ground
    /// otherwise, or the canvas where the theme claims no ground of its own.
    ///
    /// Never the *app* background and never a gradient sample: those belong to
    /// SSHub's surfaces, and `fire` is in the matrix precisely because its
    /// canvas, its app background and its PTY ground are three distinguishable
    /// colours.
    fn allowed_pty_background(app: &App, transparent: bool) -> ratatui::style::Color {
        if transparent {
            return ratatui::style::Color::Reset;
        }
        let semantic = app.theme().semantic();
        if semantic.pty_background != ratatui::style::Color::Reset {
            semantic.pty_background
        } else {
            semantic.canvas
        }
    }

    #[test]
    fn the_splash_fade_never_blends_the_remote_pty() {
        // `dashboard_at` is set once when the event loop starts and only time
        // ends the 360 ms fade, so switching to an already-open session inside
        // that window runs the *dashboard's* fade over a session frame.
        for (theme_id, _dir) in explicit_background_themes() {
            for transparent in [false, true] {
                let (mut app, _dir) = app_on(theme_id);
                app.config.appearance.transparent_session_background = transparent;
                enter_live_session(&mut app);
                write_remote_marker(&mut app);
                app.dashboard_at = Some(std::time::Instant::now());
                assert!(app.motion_enabled(), "the fade needs motion");

                let buffer = render_to_buffer(&app, 80, 24);
                let (_, row) = remote_marker_cell(buffer.area);
                let at = (find_remote_marker(&buffer, row), row);
                assert_remote_cell_untouched_by_theme(
                    &buffer,
                    at,
                    allowed_pty_background(&app, transparent),
                    &format!("splash fade, {theme_id}, transparent={transparent}"),
                );
            }
        }
    }

    #[test]
    fn the_session_exit_slide_never_repaints_the_remote_pty() {
        // The exit frame's mode is already the dashboard, so nothing about the
        // *mode* says a session is still on screen — but its snapshot is, and
        // the background pass runs after the blit that puts it there.
        for (theme_id, _dir) in explicit_background_themes() {
            for transparent in [false, true] {
                let (mut app, _dir) = app_on(theme_id);
                app.config.appearance.transparent_session_background = transparent;
                let area = Rect::new(0, 0, 80, 24);
                let pty = crate::session::render::remote_pty_rect(area);
                *app.session_snapshot.borrow_mut() = Some(remote_snapshot(area));
                // Half way through the slide, so the snapshot is genuinely
                // shifted and part of the row is dashboard again.
                app.session_exit_at = Some(
                    std::time::Instant::now()
                        .checked_sub(SESSION_ANIM / 2)
                        .unwrap(),
                );

                let buffer = render_to_buffer(&app, 80, 24);
                let at = (find_remote_marker(&buffer, pty.y), pty.y);
                assert!(at.0 > 0, "the snapshot did not travel");
                assert_remote_cell_untouched_by_theme(
                    &buffer,
                    at,
                    allowed_pty_background(&app, transparent),
                    &format!("exit slide, {theme_id}, transparent={transparent}"),
                );
                // The dashboard the slide is revealing, on the same row, is
                // still painted — the protection must be the travelling band,
                // not the whole row.
                assert_ne!(
                    buffer[(0, pty.y)].bg,
                    ratatui::style::Color::Reset,
                    "{theme_id}: the revealed dashboard was left unpainted"
                );
            }
        }
    }

    /// A session snapshot carrying one remote cell: RGB foreground, `Reset`
    /// background, at the top-left of the PTY viewport.
    fn remote_snapshot(area: Rect) -> crate::app::SessionSnapshot {
        let mut snapshot = Buffer::empty(area);
        let pty = crate::session::render::remote_pty_rect(area);
        let cell = snapshot.cell_mut((pty.x, pty.y)).unwrap();
        cell.set_symbol("R");
        cell.fg = REMOTE_FG;
        crate::app::SessionSnapshot {
            buffer: snapshot,
            remote_pty: Some(pty),
        }
    }

    fn running_to_connecting_tab_slide(
        theme_id: &str,
        elapsed: std::time::Duration,
    ) -> (App, std::time::Instant, (u16, u16)) {
        let mut app = app_with_two_sessions();
        app.activate_theme(theme_id);
        app.config.appearance.transparent_sshub_background = false;
        app.mode = AppMode::Connecting;
        app.active_session = Some(1);
        let started = std::time::Instant::now();
        app.sessions[0].phase = crate::session::SessionPhase::Running {
            started_at: started,
        };
        app.sessions[1].phase = crate::session::SessionPhase::Connecting {
            started_at: started,
        };
        app.session_tab_switch = Some(crate::app::SessionTabSwitch {
            dir: 1,
            from: 0,
            at: started,
        });

        let area = Rect::new(0, 0, 80, 24);
        let pty = crate::session::render::remote_pty_rect(area);
        let source_x = pty.right() - 2;
        let mut snapshot = Buffer::empty(area);
        let marker = snapshot.cell_mut((source_x, pty.y)).unwrap();
        marker.set_symbol("R");
        marker.fg = REMOTE_FG;
        *app.session_snapshot.borrow_mut() = Some(crate::app::SessionSnapshot {
            buffer: snapshot,
            remote_pty: Some(pty),
        });

        let p = elapsed.as_secs_f32() / TAB_ANIM.as_secs_f32();
        let travelled = (tween::ease_out(p) * pty.width as f32).round() as u16;
        (
            app,
            started + elapsed,
            (source_x.saturating_sub(travelled), pty.y),
        )
    }

    #[test]
    fn a_running_to_connecting_tab_slide_protects_the_blitted_remote_cell() {
        let elapsed = TAB_ANIM / 2;
        let (app, now, expected) = running_to_connecting_tab_slide("aqua", elapsed);

        let buffer = render_frame_at(&app, 80, 24, now);

        assert_eq!(
            buffer[expected].symbol(),
            "R",
            "the frame clock drives the blit"
        );
        assert_remote_cell_untouched_by_theme(
            &buffer,
            expected,
            // The travelling cell is backed like the resting viewport: `aqua`
            // paints its own ground, so the band carries it too.
            allowed_pty_background(&app, false),
            "running to connecting tab slide",
        );
    }

    fn app_background_tab_slide(
        outgoing: crate::session::SessionPhase,
        incoming: crate::session::SessionPhase,
    ) -> (App, std::time::Instant) {
        let mut app = app_with_two_sessions();
        wear(
            &mut app,
            "[semantic]\nbackground = \"#112233\"\n[components.session]\nbackground = \"terminal\"\n",
        );
        app.sessions[0].phase = outgoing;
        app.sessions[1].phase = incoming;
        app.active_session = Some(0);
        app.mode = match &app.sessions[0].phase {
            crate::session::SessionPhase::Connecting { .. } => AppMode::Connecting,
            _ => AppMode::Session,
        };
        let started = std::time::Instant::now();
        let _ = render_frame_at(&app, 80, 24, started);

        app.active_session = Some(1);
        app.mode = match &app.sessions[1].phase {
            crate::session::SessionPhase::Connecting { .. } => AppMode::Connecting,
            _ => AppMode::Session,
        };
        app.session_tab_switch = Some(crate::app::SessionTabSwitch {
            dir: 1,
            from: 0,
            at: started,
        });
        (app, started + TAB_ANIM / 2)
    }

    #[test]
    fn running_to_connecting_tab_slide_paints_the_incoming_chrome() {
        let started = std::time::Instant::now();
        let (app, now) = app_background_tab_slide(
            crate::session::SessionPhase::Running {
                started_at: started,
            },
            crate::session::SessionPhase::Connecting {
                started_at: started,
            },
        );

        let buffer = render_frame_at(&app, 80, 24, now);

        assert_eq!(
            buffer[(60, 3)].bg,
            Color::Rgb(0x11, 0x22, 0x33),
            "the shifted incoming Connecting chrome keeps the app background"
        );
    }

    #[test]
    fn connecting_to_running_tab_slide_paints_the_outgoing_chrome() {
        let started = std::time::Instant::now();
        let (app, now) = app_background_tab_slide(
            crate::session::SessionPhase::Connecting {
                started_at: started,
            },
            crate::session::SessionPhase::Running {
                started_at: started,
            },
        );

        let buffer = render_frame_at(&app, 80, 24, now);

        assert_eq!(
            buffer[(5, 3)].bg,
            Color::Rgb(0x11, 0x22, 0x33),
            "the shifted outgoing Connecting chrome keeps the app background"
        );
    }

    #[test]
    fn a_finished_session_tab_slide_neither_blits_nor_protects_a_remote_band() {
        let (app, now, _) = running_to_connecting_tab_slide("aqua", TAB_ANIM);
        let buffer = render_frame_at(&app, 80, 24, now);
        let pty = crate::session::render::remote_pty_rect(buffer.area);

        assert_no_remote_cells(&buffer, "finished tab slide");
        assert_ne!(
            buffer[(pty.x, pty.y)].bg,
            Color::Reset,
            "the finished slide must not leave a stale protected band"
        );
    }

    #[test]
    fn a_dashboard_tab_switch_does_not_expand_an_exit_slides_protected_band() {
        let width = 80u16;
        let area = Rect::new(0, 0, width, 24);
        let pty = crate::session::render::remote_pty_rect(area);
        let elapsed = std::time::Duration::from_millis(23);
        let (mut app, _dir, now) = exiting_app("fire", area, pty.y, elapsed);
        app.session_tab_switch = Some(crate::app::SessionTabSwitch {
            dir: 1,
            from: 0,
            at: now.checked_sub(TAB_ANIM / 2).unwrap(),
        });

        let buffer = render_frame_at(&app, width, area.height, now);
        let revealed_right = pty.x + exit_offset_at(width, elapsed);
        assert!(
            revealed_right > pty.x,
            "the exit must reveal dashboard cells"
        );

        for y in pty.y..pty.bottom() {
            for x in pty.x..revealed_right {
                assert_ne!(
                    buffer[(x, y)].bg,
                    Color::Reset,
                    "dashboard cell ({x}, {y}) outside the visible exit snapshot was protected by an unrendered tab switch"
                );
            }
        }
    }

    // ── The exit transition, with time and terminal size controlled ──
    //
    // An exit frame is *composed*: the dashboard is drawn, a session snapshot is
    // blitted over it, and only then does the background pass run. Both the
    // instant the slide is at and the geometry the snapshot came from have to be
    // the same for the blit and for the protection, or the leading remote
    // columns fall outside the exclusion.

    fn render_frame_at(app: &App, width: u16, height: u16, now: std::time::Instant) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_with_transition_clock(frame, app, now))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// A dashboard app mid-exit: `theme_id` live, a session snapshot of
    /// `snapshot` geometry carrying a remote marker on `marker_row`, and the
    /// slide armed so that `elapsed` has passed at the injected instant.
    fn exiting_app(
        theme_id: &str,
        snapshot: Rect,
        marker_row: u16,
        elapsed: std::time::Duration,
    ) -> (App, Option<tempfile::TempDir>, std::time::Instant) {
        let (mut app, dir) = app_on(theme_id);
        let started = std::time::Instant::now();
        let mut buffer = Buffer::empty(snapshot);
        let cell = buffer.cell_mut((snapshot.x, marker_row)).unwrap();
        cell.set_symbol("R");
        cell.fg = REMOTE_FG;
        *app.session_snapshot.borrow_mut() = Some(crate::app::SessionSnapshot {
            buffer,
            remote_pty: Some(crate::session::render::remote_pty_rect(snapshot)),
        });
        app.session_exit_at = Some(started);
        (app, dir, started + elapsed)
    }

    /// Where the blit puts a snapshot column at `elapsed` into the slide.
    fn exit_offset_at(width: u16, elapsed: std::time::Duration) -> u16 {
        let p = elapsed.as_secs_f32() / SESSION_ANIM.as_secs_f32();
        (tween::ease_out(p) * width as f32).round() as u16
    }

    #[test]
    fn the_exit_slide_protects_the_columns_it_actually_blitted() {
        // The defect this pins: the blit and the protection each sampled their
        // own `Instant::now()`, so the protected band could start a column or
        // more to the right of the cells the blit had already put down.
        // `elapsed` is picked so the offset lands on a rounding boundary, where
        // even a fraction of a millisecond between the two samples moves it.
        let width = 80u16;
        let area = Rect::new(0, 0, width, 24);
        let pty = crate::session::render::remote_pty_rect(area);
        for elapsed_ms in [1u64, 7, 23, 60, 140] {
            let elapsed = std::time::Duration::from_millis(elapsed_ms);
            for (theme_id, _) in explicit_background_themes() {
                let (app, _dir, now) = exiting_app(theme_id, area, pty.y, elapsed);
                let buffer = render_frame_at(&app, width, 24, now);
                let expected_x = pty.x + exit_offset_at(width, elapsed);
                assert_eq!(
                    buffer[(expected_x, pty.y)].symbol(),
                    "R",
                    "{theme_id}: the blit ignored the frame clock at {elapsed_ms}ms"
                );
                assert_remote_cell_untouched_by_theme(
                    &buffer,
                    (expected_x, pty.y),
                    allowed_pty_background(&app, false),
                    &format!("exit at {elapsed_ms}ms, {theme_id}"),
                );
            }
        }
    }

    #[test]
    fn the_exit_slide_is_protected_across_its_last_half_column_step() {
        // The rounding contract, stood on the actual threshold. Cubic ease-out
        // over 280 ms on an 80-column frame puts the last half-column step
        // between 228 ms (raw 79.4876 -> 79, the last visible column) and
        // 229 ms (raw 79.5166 -> 80, one past the right edge). Both facts are
        // asserted before anything is claimed about protection, so this test
        // cannot quietly stop straddling the boundary the way a curve change
        // would otherwise let it.
        let width = 80u16;
        let area = Rect::new(0, 0, width, 24);
        let pty = crate::session::render::remote_pty_rect(area);
        let last_visible = std::time::Duration::from_millis(228);
        let first_gone = std::time::Duration::from_millis(229);

        let off_visible = exit_offset_at(width, last_visible);
        let off_gone = exit_offset_at(width, first_gone);
        assert_ne!(
            off_visible, off_gone,
            "228ms and 229ms must land on opposite sides of a half-column step"
        );
        assert_eq!(off_visible, width - 1, "228ms is the last visible column");
        assert_eq!(off_gone, width, "229ms has left the frame");

        // The last frame that genuinely blits something — asserted
        // unconditionally, because a guarded assertion here is how the previous
        // version of this test came to check nothing at all.
        for (theme_id, _) in explicit_background_themes() {
            let (app, _dir, now) = exiting_app(theme_id, area, pty.y, last_visible);
            let buffer = render_frame_at(&app, width, 24, now);
            assert_remote_cell_untouched_by_theme(
                &buffer,
                (pty.x + off_visible, pty.y),
                allowed_pty_background(&app, false),
                &format!("exit, last visible column, {theme_id}"),
            );
        }

        // One millisecond later the slide has left the screen: nothing remote is
        // on the frame, and the band that was protected must be painted again
        // rather than left carved out of the dashboard.
        for (theme_id, _) in explicit_background_themes() {
            let (app, _dir, now) = exiting_app(theme_id, area, pty.y, first_gone);
            let buffer = render_frame_at(&app, width, 24, now);
            assert_no_remote_cells(&buffer, theme_id);
            assert_band_is_painted(&buffer, pty, theme_id);
        }
    }

    #[test]
    fn a_finished_exit_slide_leaves_no_protected_band_behind() {
        // Past `SESSION_ANIM` there is no slide at all: no blit, no exclusion.
        let width = 80u16;
        let area = Rect::new(0, 0, width, 24);
        let pty = crate::session::render::remote_pty_rect(area);
        for (theme_id, _) in explicit_background_themes() {
            let (app, _dir, now) = exiting_app(theme_id, area, pty.y, SESSION_ANIM);
            let buffer = render_frame_at(&app, width, 24, now);
            assert_no_remote_cells(&buffer, theme_id);
            assert_band_is_painted(&buffer, pty, theme_id);
        }
    }

    fn assert_chrome_exit_slide_is_painted(phase: crate::session::SessionPhase, what: &str) {
        let mut app = app_with_two_sessions();
        wear(
            &mut app,
            "[semantic]\nbackground = \"#112233\"\n[components.session]\nbackground = \"terminal\"\n",
        );
        app.sessions[0].phase = phase;
        app.active_session = Some(0);
        app.mode = match &app.sessions[0].phase {
            crate::session::SessionPhase::Connecting { .. } => AppMode::Connecting,
            _ => AppMode::Session,
        };
        let started = std::time::Instant::now();
        let _ = render_frame_at(&app, 80, 24, started);

        app.mode = AppMode::Normal;
        app.session_exit_at = Some(started);
        let buffer = render_frame_at(&app, 80, 24, started + SESSION_ANIM / 2);
        let offset = exit_offset_at(80, SESSION_ANIM / 2);

        for x in offset..80 {
            assert_eq!(
                buffer[(x, 3)].bg,
                Color::Rgb(0x11, 0x22, 0x33),
                "{what}: visible chrome at ({x}, 3) lost the app background"
            );
        }
    }

    #[test]
    fn connecting_exit_slide_paints_the_outgoing_chrome() {
        assert_chrome_exit_slide_is_painted(
            crate::session::SessionPhase::Connecting {
                started_at: std::time::Instant::now(),
            },
            "Connecting exit",
        );
    }

    #[test]
    fn failed_exit_slide_paints_the_outgoing_chrome() {
        assert_chrome_exit_slide_is_painted(
            crate::session::SessionPhase::Exited {
                status: "failed".into(),
                at: std::time::Instant::now(),
            },
            "failed exit",
        );
    }

    /// No cell anywhere carries the remote marker's colour.
    fn assert_no_remote_cells(buffer: &Buffer, what: &str) {
        let a = buffer.area;
        for y in a.y..a.bottom() {
            for x in a.x..a.right() {
                assert_ne!(
                    buffer[(x, y)].fg,
                    REMOTE_FG,
                    "{what}: remote output is still on screen at ({x}, {y})"
                );
            }
        }
    }

    /// Every cell of `band` got a background — i.e. no stale exclusion is
    /// keeping the theme off a region the slide no longer covers.
    fn assert_band_is_painted(buffer: &Buffer, band: Rect, what: &str) {
        for y in band.y..band.bottom() {
            for x in band.x..band.right() {
                assert_ne!(
                    buffer[(x, y)].bg,
                    ratatui::style::Color::Reset,
                    "{what}: ({x}, {y}) was left unpainted after the slide ended"
                );
            }
        }
    }

    #[test]
    fn the_splash_fade_and_the_exit_slide_agree_on_the_same_frame() {
        // The fade computed the protection a third time. With both running, an
        // exposed remote cell loses its background *and* its foreground.
        let width = 80u16;
        let area = Rect::new(0, 0, width, 24);
        let pty = crate::session::render::remote_pty_rect(area);
        let elapsed = std::time::Duration::from_millis(23);
        for (theme_id, _) in explicit_background_themes() {
            let (mut app, _dir, now) = exiting_app(theme_id, area, pty.y, elapsed);
            app.dashboard_at = Some(now);
            let buffer = render_frame_at(&app, width, 24, now);
            let expected_x = pty.x + exit_offset_at(width, elapsed);
            assert_remote_cell_untouched_by_theme(
                &buffer,
                (expected_x, pty.y),
                allowed_pty_background(&app, false),
                &format!("exit under the splash fade, {theme_id}"),
            );
        }
    }

    #[test]
    fn an_exit_snapshot_from_a_taller_terminal_is_protected_where_it_lands() {
        // The terminal can shrink mid-slide: SSHub resizes live and does not
        // drop the snapshot. Rows that were the old PTY body then land on rows
        // that are now footer or dashboard — outside the *current* viewport,
        // and paintable unless the protection comes from the snapshot.
        let snapshot = Rect::new(0, 0, 80, 24);
        let old_pty = crate::session::render::remote_pty_rect(snapshot);
        let shrunk = Rect::new(0, 0, 80, 20);
        let new_pty = crate::session::render::remote_pty_rect(shrunk);
        // The last row the shrunk terminal still shows: inside the old PTY body,
        // but the *new* frame's footer row — so the current viewport does not
        // cover it and only a snapshot-derived region can.
        let marker_row = shrunk.bottom() - 1;
        assert!(
            marker_row < old_pty.bottom() && marker_row >= new_pty.bottom(),
            "the marker must be old remote output landing outside the new viewport"
        );
        let elapsed = std::time::Duration::from_millis(23);

        for (theme_id, _) in explicit_background_themes() {
            let (app, _dir, now) = exiting_app(theme_id, snapshot, marker_row, elapsed);
            let buffer = render_frame_at(&app, shrunk.width, shrunk.height, now);
            let expected_x = old_pty.x + exit_offset_at(shrunk.width, elapsed);
            assert_remote_cell_untouched_by_theme(
                &buffer,
                (expected_x, marker_row),
                allowed_pty_background(&app, false),
                &format!("exit after a shrink, {theme_id}"),
            );
        }
    }

    #[test]
    fn an_exit_snapshot_from_a_shorter_terminal_protects_only_what_it_covers() {
        // The mirror case: a grown terminal must not have the rows the snapshot
        // never reached carved out of the dashboard.
        let snapshot = Rect::new(0, 0, 80, 20);
        let grown = Rect::new(0, 0, 80, 24);
        let old_pty = crate::session::render::remote_pty_rect(snapshot);
        let elapsed = std::time::Duration::from_millis(23);
        let (app, _dir, now) = exiting_app("fire", snapshot, old_pty.y, elapsed);
        let buffer = render_frame_at(&app, grown.width, grown.height, now);

        for row in old_pty.bottom()..grown.height {
            assert_ne!(
                buffer[(grown.width - 1, row)].bg,
                ratatui::style::Color::Reset,
                "row {row} is below the snapshot and must still be painted"
            );
        }
    }

    #[test]
    fn a_session_in_the_background_protects_nothing_on_the_dashboard() {
        // The protected rect is claimed from the frame that actually drew the
        // grid. An open session the user has stepped away from must not carve
        // an unpainted band out of the dashboard.
        let mut app = session_app_with_theme("aqua");
        app.mode = AppMode::Normal;
        app.config.appearance.transparent_sshub_background = false;
        let buffer = render_to_buffer(&app, 80, 24);
        assert_all_backgrounds_non_reset(&buffer);
    }

    /// Every cell outside the remote grid, i.e. SSHub's own surfaces.
    fn sshub_surface_cells(buffer: &Buffer, in_session: bool) -> Vec<(u16, u16)> {
        let pty = in_session.then(|| crate::session::render::remote_pty_rect(buffer.area));
        (buffer.area.y..buffer.area.bottom())
            .flat_map(|y| (buffer.area.x..buffer.area.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| !pty.is_some_and(|r| r.contains((x, y).into())))
            .collect()
    }

    #[test]
    fn sshub_surfaces_are_opaque_out_of_the_box_under_every_theme() {
        // The shipped state is opaque; transparency is the user's explicit
        // choice. A theme that leaves its ground to the emulator therefore
        // still gets a filled app, through the canvas.
        for theme in ["default", "fire", "summer", "aqua", "high-contrast"] {
            let app = app_with_builtin_theme(theme);
            let buffer = render_to_buffer(&app, 80, 24);
            let bare: Vec<_> = sshub_surface_cells(&buffer, false)
                .into_iter()
                .filter(|&(x, y)| buffer[(x, y)].bg == ratatui::style::Color::Reset)
                .collect();
            assert!(
                bare.is_empty(),
                "{theme}: {} cells stayed transparent by default, starting at {:?}",
                bare.len(),
                bare.first()
            );
        }
    }

    #[test]
    fn the_user_can_make_sshubs_own_surfaces_transparent_whatever_the_theme_paints() {
        for theme in ["default", "fire", "summer", "aqua"] {
            let mut app = app_with_builtin_theme(theme);
            app.config.appearance.transparent_sshub_background = true;
            let buffer = render_to_buffer(&app, 80, 24);
            let freed = sshub_surface_cells(&buffer, false)
                .into_iter()
                .filter(|&(x, y)| buffer[(x, y)].bg == ratatui::style::Color::Reset)
                .count();
            // How much comes free depends on the theme, because a widget only
            // draws a panel body where the theme gave it one: `default` paints
            // no surfaces at all, the others do and have them released too (see
            // `releasing_sshub_reaches_the_panel_bodies_a_theme_paints_itself`).
            // The lower bound is what every theme has in common — the ground the
            // pass above would otherwise have laid down.
            assert!(
                freed > 300,
                "{theme}: only {freed} cells went transparent, expected the app ground"
            );
        }
    }

    #[test]
    fn releasing_sshub_reaches_the_panel_bodies_a_theme_paints_itself() {
        // `fire` fills its panels through `semantic.surface`, and the widgets
        // draw that themselves — more cells than the ground pass ever covers.
        // Releasing only what the pass painted would leave the dashboard as
        // brown as before, which is not what the switch promises.
        let opaque = app_with_builtin_theme("fire");
        let grounds = {
            let s = opaque.theme().semantic();
            [s.background, s.canvas, s.surface, s.surface_raised]
        };
        let mut app = app_with_builtin_theme("fire");
        app.config.appearance.transparent_sshub_background = true;
        let buffer = render_to_buffer(&app, 80, 24);
        for ground in grounds {
            assert_ne!(ground, ratatui::style::Color::Reset);
            let left: Vec<_> = sshub_surface_cells(&buffer, false)
                .into_iter()
                .filter(|&(x, y)| buffer[(x, y)].bg == ground)
                .collect();
            assert!(
                left.is_empty(),
                "{} cells still carry the ground {ground:?}, starting at {:?}",
                left.len(),
                left.first()
            );
        }
    }

    /// `with_ground_released` names one style recipe literally
    /// (`TextOnSurfaceRaised`). A recipe added later that also seats its text on
    /// a ground slot would be missed in silence: the match's wildcard arm simply
    /// would not fire, and a panel body would stay opaque with no test to say so.
    ///
    /// So the claim is checked against the release itself, over the roles the
    /// catalogue really has — not over a hand-kept list of recipes, which would
    /// have the same blind spot as the code it is meant to guard.
    #[test]
    fn every_role_seated_on_a_ground_slot_is_actually_released() {
        use crate::theme::catalog::{RoleFallback, RoleRef, ROLE_SPECS};

        let app = app_with_builtin_theme("fire");
        let base = app.base_theme();
        let released = base.with_ground_released();
        let grounds = {
            let s = base.semantic();
            [s.background, s.canvas, s.surface, s.surface_raised]
        };
        for ground in grounds {
            assert_ne!(ground, Color::Reset, "fire must paint every ground slot");
        }

        let mut checked = 0;
        for spec in ROLE_SPECS {
            let RoleRef::Style(role) = spec.role else {
                continue;
            };
            let RoleFallback::Style(recipe) = spec.fallback else {
                continue;
            };
            // Does this role's *fallback* seat its text on a ground?
            let seated = crate::theme::model::semantic_style(base.semantic(), recipe)
                .bg
                .is_some_and(|bg| grounds.contains(&bg));
            if !seated {
                continue;
            }
            checked += 1;
            assert_eq!(
                released.style(role).bg,
                Some(Color::Reset),
                "{}: seated on a ground slot but still opaque after the release — \
                 add its recipe to the match in `with_ground_released`",
                spec.path
            );
        }
        assert!(
            checked > 0,
            "no role was seated on a ground slot; this test stopped proving anything"
        );
    }

    #[test]
    fn a_selection_sharing_the_surface_colour_survives_the_release() {
        // The case a colour comparison over the finished frame cannot get
        // right: a theme may legitimately give `selection_bg` and `surface` the
        // same value, and after the release the two are no longer alike — one
        // is wallpaper, the other still has to mark the selected row. Only the
        // catalogue fallback can tell them apart, and it does so before
        // anything is drawn.
        let mut app = app_with_builtin_theme("fire");
        wear(
            &mut app,
            "[semantic]\nsurface = \"#2b2b2b\"\nselection_bg = \"#2b2b2b\"\n",
        );
        app.config.appearance.transparent_sshub_background = true;
        let buffer = render_to_buffer(&app, 80, 24);
        let bar = sshub_surface_cells(&buffer, false)
            .into_iter()
            .filter(|&(x, y)| buffer[(x, y)].bg == ratatui::style::Color::Rgb(0x2b, 0x2b, 0x2b))
            .count();
        assert!(
            bar > 0,
            "the selection bar was released along with the surface it shares a colour with"
        );
    }

    #[test]
    fn releasing_sshub_keeps_the_colours_that_are_not_ground() {
        // Selection bars, status colours and inverted chrome are *drawing*, not
        // ground: a see-through dashboard still has to show which row is
        // selected.
        let mut app = app_with_builtin_theme("fire");
        app.config.appearance.transparent_sshub_background = true;
        let buffer = render_to_buffer(&app, 80, 24);
        let semantic = app.theme().semantic();
        let selection = sshub_surface_cells(&buffer, false)
            .into_iter()
            .filter(|&(x, y)| buffer[(x, y)].bg == semantic.selection_bg)
            .count();
        assert!(
            selection > 0,
            "the selected row lost its bar along with the ground"
        );
    }

    #[test]
    fn the_two_transparency_switches_are_independent() {
        let mut app = session_app_with_theme("fire");
        app.config.appearance.transparent_sshub_background = true;
        app.config.appearance.transparent_session_background = false;
        let buffer = render_to_buffer(&app, 80, 24);
        assert_eq!(
            buffer[remote_pty_probe(buffer.area)].bg,
            app.theme().semantic().pty_background,
            "the grid keeps its ground while only the app went transparent"
        );

        app.config.appearance.transparent_sshub_background = false;
        app.config.appearance.transparent_session_background = true;
        let buffer = render_to_buffer(&app, 80, 24);
        assert_eq!(
            buffer[remote_pty_probe(buffer.area)].bg,
            ratatui::style::Color::Reset,
            "the grid went transparent while the app kept its ground"
        );
        let bare = sshub_surface_cells(&buffer, true)
            .into_iter()
            .filter(|&(x, y)| buffer[(x, y)].bg == ratatui::style::Color::Reset)
            .count();
        assert_eq!(bare, 0, "the app surfaces must stay opaque");
    }

    #[test]
    fn the_canvas_pair_backs_a_grid_no_theme_claims() {
        // `default` leaves its ground to the emulator, so the grid has no themed
        // pair of its own and the canvas/text fallback is what keeps it opaque.
        // Read from the theme as authored: `app.theme()` would hand back the
        // released view here, whose canvas is `Reset` — comparing against that
        // would accept a see-through grid as the expected value.
        let mut app = session_app_with_theme("default");
        let base = {
            let s = app.base_theme().semantic();
            (s.pty_background, s.canvas, s.text)
        };
        assert_eq!(
            base.0,
            ratatui::style::Color::Reset,
            "this test needs a theme without a PTY ground"
        );
        assert_ne!(
            base.1,
            ratatui::style::Color::Reset,
            "the canvas must be opaque"
        );

        app.config.appearance.transparent_sshub_background = true;
        let buffer = render_to_buffer(&app, 80, 24);
        let probe = remote_pty_probe(buffer.area);
        assert_eq!(
            (buffer[probe].bg, buffer[probe].fg),
            (base.1, base.2),
            "the grid must carry the authored canvas pair"
        );
    }

    #[test]
    fn the_user_can_hand_the_grid_back_to_the_terminal_whatever_the_theme_paints() {
        // The grid is the largest surface on screen, so releasing it is what
        // makes a terminal wallpaper visible again — and it is a decision of
        // its own, separate from SSHub's surfaces around it.
        let mut app = session_app_with_theme("fire");
        app.config.appearance.transparent_session_background = true;
        let buffer = render_to_buffer(&app, 80, 24);
        let probe = remote_pty_probe(buffer.area);
        assert_eq!(
            (buffer[probe].bg, buffer[probe].fg),
            (ratatui::style::Color::Reset, ratatui::style::Color::Reset),
            "both channels must go back to the emulator, or its opacity setting \
             cannot reach the grid"
        );
        // The app's own chrome is the theme's business and stays painted.
        assert_ne!(buffer[(1, 0)].bg, ratatui::style::Color::Reset);
    }

    #[test]
    fn releasing_the_grid_leaves_the_canvas_fill_its_own_work() {
        // `default` is where the division of labour is visible: the grid goes
        // back to the emulator, and the canvas still fills everything else.
        let mut app = session_app_with_theme("default");
        app.config.appearance.transparent_session_background = true;
        let buffer = render_to_buffer(&app, 80, 24);
        let pty = crate::session::render::remote_pty_rect(buffer.area);
        assert_eq!(
            buffer[remote_pty_probe(buffer.area)].bg,
            ratatui::style::Color::Reset,
            "the newer, more specific switch owns the grid"
        );
        let outside_left_unpainted: Vec<_> = (buffer.area.y..buffer.area.bottom())
            .flat_map(|y| (buffer.area.x..buffer.area.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| !pty.contains((x, y).into()))
            .filter(|&(x, y)| buffer[(x, y)].bg == ratatui::style::Color::Reset)
            .collect();
        assert!(
            outside_left_unpainted.is_empty(),
            "the switch still fills everything outside the grid, but {} cells \
             stayed transparent, starting at {:?}",
            outside_left_unpainted.len(),
            outside_left_unpainted.first()
        );
    }

    #[test]
    fn releasing_sshubs_surfaces_leaves_a_themed_grid_untouched() {
        // `fire` is the built-in whose canvas differs from its background, so a
        // ground that leaked from SSHub's side would be visible cell by cell.
        let mut app = session_app_with_theme("fire");
        app.config.appearance.transparent_sshub_background = true;
        let semantic = app.base_theme().semantic();
        assert_ne!(
            semantic.pty_background, semantic.canvas,
            "this test needs a theme whose PTY ground and canvas differ"
        );
        let buffer = render_to_buffer(&app, 80, 24);
        let probe = remote_pty_probe(buffer.area);
        assert_eq!(
            buffer[probe].bg, semantic.pty_background,
            "the grid keeps the ground its theme gave it; only its own switch releases it"
        );
    }

    #[test]
    fn a_themed_pty_ground_keeps_the_colours_the_remote_chose() {
        let mut app = session_app_with_theme("summer");
        app.config.appearance.transparent_sshub_background = false;
        write_remote_marker(&mut app);
        let buffer = render_to_buffer(&app, 80, 24);
        let (_, row) = remote_marker_cell(buffer.area);
        let at = (find_remote_marker(&buffer, row), row);
        assert_eq!(
            buffer[at].fg, REMOTE_FG,
            "the remote picked this foreground; the ground pass must not take it"
        );
    }

    #[test]
    fn a_pty_theme_opt_out_falls_back_to_the_canvas_not_to_the_emulator() {
        // `pty_background = "terminal"` drops the theme's own grid colour, but
        // it cannot make the grid see-through: SSHub is opaque out of the box,
        // and releasing it is the user's call through the Settings switch.
        let mut app = app_with_builtin_theme("summer");
        wear(&mut app, "[semantic]\npty_background = \"terminal\"\n");
        enter_live_session(&mut app);
        let buffer = render_to_buffer(&app, 80, 24);
        let probe = remote_pty_probe(buffer.area);
        let semantic = app.theme().semantic();
        assert_eq!(
            (buffer[probe].bg, buffer[probe].fg),
            (semantic.canvas, semantic.text),
            "the opt-out falls back to the canvas pair"
        );
    }

    /// An app mid-way into the enter slide, with a dashboard snapshot behind it.
    ///
    /// The theme gets a PTY ground of its own, distinct from every other colour
    /// it paints, so a cell carrying it can only have come from the grid pass.
    fn session_entering_at(
        theme_id: &str,
        elapsed: std::time::Duration,
    ) -> (App, std::time::Instant, u16) {
        let mut app = app_with_builtin_theme(theme_id);
        wear(&mut app, "[semantic]\npty_background = \"#123456\"\n");
        enter_live_session(&mut app);
        let area = Rect::new(0, 0, 80, 24);
        *app.dashboard_snapshot.borrow_mut() = Some(Buffer::empty(area));
        let started = std::time::Instant::now();
        app.session_enter_at = Some(started);
        assert!(app.motion_enabled(), "the slide needs motion");
        let p = elapsed.as_secs_f32() / SESSION_ANIM.as_secs_f32();
        let off = ((1.0 - tween::ease_out(p)) * area.width as f32).round() as u16;
        (app, started + elapsed, off)
    }

    #[test]
    fn the_enter_slide_owns_the_right_columns_at_every_point_of_its_travel() {
        for step in [1u32, 4, 8, 15, 19] {
            let elapsed = SESSION_ANIM * step / 20;
            let (app, now, off) = session_entering_at("fire", elapsed);
            let buffer = render_frame_at(&app, 80, 24, now);
            let pty = crate::session::render::remote_pty_rect(buffer.area);
            let ground = app.theme().semantic().pty_background;
            let ahead: Vec<_> = (pty.y..pty.bottom())
                .flat_map(|y| (pty.x..pty.x + off).map(move |x| (x, y)))
                .filter(|&(x, y)| buffer[(x, y)].bg == ground)
                .collect();
            assert!(
                ahead.is_empty(),
                "step {step}/20 (off={off}): {} cells ahead of the session carry \
                 the grid ground, starting at {:?}",
                ahead.len(),
                ahead.first()
            );
        }
    }

    #[test]
    fn a_slide_that_cannot_play_protects_the_resting_viewport() {
        // No usable dashboard behind it means no blit — and then the grid sits
        // where it rests, so that is what ownership has to cover.
        let (app, now, _) = session_entering_at("fire", SESSION_ANIM / 2);
        *app.dashboard_snapshot.borrow_mut() = None;
        let buffer = render_frame_at(&app, 80, 24, now);
        let probe = remote_pty_probe(buffer.area);
        assert_eq!(
            buffer[probe].bg,
            app.theme().semantic().pty_background,
            "the resting grid lost its ground"
        );
    }

    #[test]
    fn the_enter_slide_protects_only_the_columns_the_remote_actually_occupies() {
        // The slide carries the session in from the right and restores the
        // dashboard on its left. The resting PTY rect therefore covers cells
        // that are dashboard this frame, and backing them with the grid's own
        // pair paints remote ground over SSHub's own surface.
        let (app, now, off) = session_entering_at("fire", SESSION_ANIM / 2);
        assert!(
            off > 0 && off < 80,
            "the slide must be mid-flight, off={off}"
        );
        let buffer = render_frame_at(&app, 80, 24, now);
        let pty = crate::session::render::remote_pty_rect(buffer.area);
        let ground = app.theme().semantic().pty_background;
        assert_eq!(ground, ratatui::style::Color::Rgb(0x12, 0x34, 0x56));

        let ahead: Vec<_> = (pty.y..pty.bottom())
            .flat_map(|y| (pty.x..pty.x + off).map(move |x| (x, y)))
            .filter(|&(x, y)| buffer[(x, y)].bg == ground)
            .collect();
        assert!(
            ahead.is_empty(),
            "{} cells left of the travelling session carry the grid's ground, \
             starting at {:?}",
            ahead.len(),
            ahead.first()
        );
    }

    #[test]
    fn a_solid_pty_ground_with_a_terminal_foreground_still_gets_a_foreground() {
        // `"terminal"` is legal for every slot but `background`, so a theme may
        // pair a painted ground with an emulator-owned foreground. Taking that
        // pair at face value re-opens the reported bug: a written background
        // over a `Reset` foreground is the emulator's near-white on our ground.
        let mut app = app_with_builtin_theme("summer");
        wear(
            &mut app,
            "[semantic]\npty_background = \"#123456\"\npty_foreground = \"terminal\"\n",
        );
        enter_live_session(&mut app);
        let buffer = render_to_buffer(&app, 80, 24);
        let probe = remote_pty_probe(buffer.area);
        assert_eq!(
            buffer[probe].bg,
            ratatui::style::Color::Rgb(0x12, 0x34, 0x56)
        );
        assert_eq!(
            buffer[probe].fg,
            app.theme().semantic().text,
            "an unusable foreground must fall back to the plain text colour"
        );
    }

    #[test]
    fn releasing_sshubs_surfaces_leaves_the_grid_opaque() {
        // The two switches are independent, and `default` is where that can
        // actually break: its grid has no ground of its own and falls back to
        // the canvas — which the released view sets to `Reset` along with every
        // other ground. The fallback has to come from the theme as authored.
        let base_canvas = app_with_builtin_theme("default").theme().semantic().canvas;
        assert_ne!(base_canvas, ratatui::style::Color::Reset);

        let mut app = session_app_with_theme("default");
        app.config.appearance.transparent_sshub_background = true;
        app.config.appearance.transparent_session_background = false;
        let buffer = render_to_buffer(&app, 80, 24);
        assert_eq!(
            buffer[remote_pty_probe(buffer.area)].bg,
            base_canvas,
            "the grid followed the wrong switch"
        );
    }

    #[test]
    fn a_theme_that_leaves_every_foreground_to_the_emulator_writes_no_ground_at_all() {
        // Last line of the pair invariant: if even the fallback resolves to
        // `Reset`, writing the background alone would hand the foreground back
        // to the emulator — the exact reported bug. Both channels or neither.
        let mut app = app_with_builtin_theme("summer");
        wear(
            &mut app,
            "[semantic]\npty_background = \"#123456\"\n\
             pty_foreground = \"terminal\"\ntext = \"terminal\"\n",
        );
        enter_live_session(&mut app);
        let buffer = render_to_buffer(&app, 80, 24);
        let half = half_painted_pty_cells(&buffer);
        assert!(
            half.is_empty(),
            "{} PTY cells carry a background over an emulator foreground, starting at {:?}",
            half.len(),
            half.first()
        );
        // …and not by inventing a *complete* pair either: with no honest
        // foreground left, the grid stays the emulator's in both channels.
        let probe = remote_pty_probe(buffer.area);
        assert_eq!(
            (buffer[probe].bg, buffer[probe].fg),
            (ratatui::style::Color::Reset, ratatui::style::Color::Reset),
            "a ground was written despite having no foreground to pair it with"
        );
    }

    #[test]
    fn a_reversed_remote_cell_gets_both_channels_or_it_swaps_against_nothing() {
        let mut app = session_app_with_theme("summer");
        app.config.appearance.transparent_sshub_background = false;
        // Reverse video with no colours of its own: the emulator swaps whatever
        // the two channels hold, so leaving one at `Reset` swaps our ground
        // against a foreground that was never defined.
        app.sessions[0].parser.process(b"\x1b[7mREVERSED");
        let buffer = render_to_buffer(&app, 80, 24);
        let pty = crate::session::render::remote_pty_rect(buffer.area);
        let cell = &buffer[(pty.x, pty.y)];
        assert!(
            cell.modifier.contains(Modifier::REVERSED),
            "the marker did not reach the grid reversed"
        );
        let semantic = app.theme().semantic();
        assert_eq!(
            (cell.bg, cell.fg),
            (semantic.pty_background, semantic.pty_foreground),
            "a reversed cell must carry the whole pair"
        );
    }

    /// Every PTY cell that ended up with one channel painted and the other left
    /// to the emulator. Painting only the background is what made `summer`
    /// unreadable: the remote's default foreground stayed `Reset`, so the
    /// emulator wrote its own near-white into our cream ground.
    fn half_painted_pty_cells(buffer: &Buffer) -> Vec<(u16, u16)> {
        use ratatui::style::Color;
        let pty = crate::session::render::remote_pty_rect(buffer.area);
        let mut found = Vec::new();
        for y in pty.y..pty.bottom() {
            for x in pty.x..pty.right() {
                let cell = &buffer[(x, y)];
                if cell.bg != Color::Reset && cell.fg == Color::Reset {
                    found.push((x, y));
                }
            }
        }
        found
    }

    #[test]
    fn a_painted_pty_ground_never_leaves_the_foreground_to_the_emulator() {
        let mut app = session_app_with_theme("summer");
        app.config.appearance.transparent_sshub_background = true;
        let buffer = render_to_buffer(&app, 80, 24);
        let half = half_painted_pty_cells(&buffer);
        assert!(
            half.is_empty(),
            "{} PTY cells carry a themed background over an emulator foreground, \
             starting at {:?}",
            half.len(),
            half.first()
        );
    }

    #[test]
    fn a_theme_with_its_own_ground_backs_the_remote_pty() {
        let mut app = session_app_with_theme("summer");
        app.config.appearance.transparent_sshub_background = false;
        let buffer = render_to_buffer(&app, 80, 24);
        let probe = remote_pty_probe(buffer.area);
        assert_eq!(
            buffer[probe].bg,
            app.theme().semantic().background,
            "a theme that paints its own ground must paint it under the PTY too"
        );
    }

    #[test]
    fn render_includes_host_name_in_list() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "web-prod"));
    }

    #[test]
    fn sshub_is_opaque_by_default_and_transparent_on_request() {
        use ratatui::style::Color;
        let mut app = test_app_with_hosts();

        // Shipped state: no cell is left to the emulator.
        let opaque = render_to_buffer(&app, 120, 38);
        let a = opaque.area;
        let all_opaque = (a.y..a.y + a.height)
            .all(|y| (a.x..a.x + a.width).all(|x| opaque[(x, y)].bg != Color::Reset));
        assert!(all_opaque, "the default state left a transparent cell");

        // Asked for: the ground goes back, so the emulator's own background —
        // and its opacity setting — reach the screen again.
        app.config.appearance.transparent_sshub_background = true;
        let transparent = render_to_buffer(&app, 120, 38);
        let a = transparent.area;
        let any_reset = (a.y..a.y + a.height)
            .any(|y| (a.x..a.x + a.width).any(|x| transparent[(x, y)].bg == Color::Reset));
        assert!(any_reset, "nothing came free with transparency on");
    }

    #[test]
    fn render_shows_host_card_and_version() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 120, 38);
        // The selected-host card (middle column) is titled "host · <name>".
        assert!(buffer_contains(&buffer, "host \u{b7} web-prod"));
        // Its address:port row is rendered.
        assert!(buffer_contains(&buffer, "10.0.0.1:22"));
        // The build version appears in the tab bar.
        let version = concat!("v", env!("CARGO_PKG_VERSION"));
        assert!(buffer_contains(&buffer, version));
    }

    #[test]
    fn overlays_do_not_panic_on_a_tiny_terminal() {
        // Regression: popup geometry used u16::clamp(min, max) with max derived
        // from the terminal size, which asserted min<=max and crashed the TUI
        // when the terminal was smaller than the popup minimum. Every overlay
        // must render without panicking even at absurdly small sizes.
        let modes = [
            AppMode::Palette,
            AppMode::GroupManage,
            AppMode::Help,
            AppMode::KeybindEditor,
            AppMode::ConfirmQuit,
            AppMode::KnownHosts,
        ];
        for &mode in &modes {
            for (w, h) in [(1u16, 1u16), (10, 3), (30, 8), (49, 20)] {
                let mut app = test_app_with_hosts();
                app.mode = mode;
                if mode == AppMode::KnownHosts {
                    app.known_hosts = Some(crate::app::KnownHostsState {
                        entries: vec![crate::known_hosts::KnownHostEntry {
                            marker: None,
                            hosts: "example.com".to_string(),
                            key_type: "ssh-ed25519".to_string(),
                            fingerprint: Some(
                                "SHA256:abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG".to_string(),
                            ),
                        }],
                        selected: 0,
                        query: String::new(),
                        confirming_delete: false,
                        notice: None,
                        notice_is_error: false,
                    });
                }
                // Must not panic; we don't care about the pixels here.
                let _ = render_to_buffer(&app, w, h);
            }
        }
        for purpose in [
            crate::app::SessionPickerPurpose::NewSession,
            crate::app::SessionPickerPurpose::SftpLeftPane,
            crate::app::SessionPickerPurpose::SwitchSession,
        ] {
            for (w, h) in [(1u16, 1u16), (10, 3), (30, 8), (49, 20)] {
                let app = app_with_picker(purpose, "x");
                let _ = render_to_buffer(&app, w, h);
            }
        }

        // The log browser's viewer + open bookmarks overlay nests a popup inside
        // the viewer's content area, whose own clamp must not assert on a sliver.
        {
            use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
            use std::io::Write;
            let tmp = tempfile::tempdir().unwrap();
            let logs = tmp.path().join("logs");
            let host_dir = logs.join("web-1");
            std::fs::create_dir_all(&host_dir).unwrap();
            std::fs::File::create(host_dir.join("100-1-0.log"))
                .unwrap()
                .write_all(b"line one\nline two\n")
                .unwrap();
            let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
            for (w, h) in [(1u16, 1u16), (10, 3), (30, 8), (49, 20)] {
                let mut app = test_app_with_hosts();
                app.open_log_browser_at(logs.clone()).unwrap();
                app.handle_key(enter).unwrap(); // host -> segments
                app.handle_key(enter).unwrap(); // segment -> viewer
                app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty()))
                    .unwrap(); // start a bookmark
                app.handle_key(enter).unwrap(); // blank name -> "line N"
                app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::empty()))
                    .unwrap(); // open the bookmarks overlay
                let _ = render_to_buffer(&app, w, h);
            }
        }
    }

    /// The picker keeps the dashboard's themed keybind footer for every purpose
    /// instead of swapping in the legacy status bar, which paints an off-theme
    /// `DarkGray` band and reads "Enter: connect" plus a host count under a
    /// session switcher. Compares the footer row cell by cell against the very
    /// same dashboard with no picker up, so a purpose-dependent footer or a
    /// restyled band both fail.
    #[test]
    fn session_picker_keeps_the_dashboard_footer() {
        fn footer_cells(app: &App) -> Vec<(String, Color, Color)> {
            let buf = render_to_buffer(app, 120, 38);
            let footer = dashboard_layout::dashboard_layout_zoomed(buf.area, app.ui_zoom).footer;
            (footer.x..footer.right())
                .map(|x| {
                    let cell = buf.cell((x, footer.y)).unwrap();
                    (cell.symbol().to_string(), cell.fg, cell.bg)
                })
                .collect()
        }

        for purpose in [
            crate::app::SessionPickerPurpose::NewSession,
            crate::app::SessionPickerPurpose::SftpLeftPane,
            crate::app::SessionPickerPurpose::SwitchSession,
        ] {
            let mut app = app_with_picker(purpose, "");
            assert_eq!(app.mode, AppMode::SessionPicker, "{purpose:?}");
            let with_picker = footer_cells(&app);

            // The very same app with the overlay dismissed — sessions, hosts and
            // tab all identical, so the open picker is the only difference the
            // footer could react to.
            app.session_picker = None;
            app.mode = AppMode::Normal;
            let without_picker = footer_cells(&app);

            assert!(
                without_picker
                    .iter()
                    .all(|(_, _, bg)| *bg != Color::DarkGray),
                "{purpose:?}: the dashboard footer is themed, not a DarkGray band"
            );
            assert_eq!(with_picker, without_picker, "{purpose:?} footer row");
        }
    }

    #[test]
    fn session_picker_renders_title_and_empty_state_per_purpose() {
        use crate::app::SessionPickerPurpose::{NewSession, SftpLeftPane, SwitchSession};

        for (purpose, title, empty) in [
            (NewSession, "new session tab", "(no matching hosts)"),
            (SftpLeftPane, "select left server", "(no matching hosts)"),
            (SwitchSession, "switch session", "(no matching sessions)"),
        ] {
            let app = app_with_picker(purpose, "zzzznope");
            let text = buffer_text(&render_to_buffer(&app, 80, 24));
            assert!(text.contains(title), "{purpose:?} title");
            assert!(text.contains(empty), "{purpose:?} empty state");
            assert!(text.contains("zzzznope"), "{purpose:?} query echoed");
        }
    }

    #[test]
    fn session_picker_renders_each_lifecycle_with_word_colour_and_ordinal() {
        // The word carries the state without colour, the colour without reading.
        // The ordinal sits at a fixed offset after the badge (BADGE_CELLS = 7)
        // and must be read there — the endpoints contain digits too, so a plain
        // `contains('1')` would prove nothing.
        //
        // Marker colours, not `default` parity: an unbound badge role would
        // reproduce the legacy colour just as faithfully as a bound one.
        let mut app = app_with_picker(crate::app::SessionPickerPurpose::SwitchSession, "");
        wear(
            &mut app,
            "[components.picker]\n\
             badge_success = \"#ff4001\"\n\
             badge_warning = \"#ff4002\"\n\
             badge_error = \"#ff4003\"\n",
        );
        let buf = render_to_buffer(&app, 80, 24);
        for (word, colour, ordinal) in [
            ("conn", Color::Rgb(0xff, 0x40, 0x02), "1"),
            ("up", Color::Rgb(0xff, 0x40, 0x01), "2"),
            ("exit", Color::Rgb(0xff, 0x40, 0x03), "3"),
        ] {
            let (x, y) = picker_row(&buf, word);
            assert_eq!(buf[(x, y)].fg, colour, "{word}: dot colour");
            assert_eq!(
                cells_at(&buf, x + 7, y, 3).trim(),
                ordinal,
                "{word}: tab ordinal"
            );
        }

        let text = buffer_text(&buf);
        assert!(text.contains("micha@10.0.0.11:22"), "endpoint rendered");
        assert!(text.contains("current"), "active session marked");
    }

    #[test]
    fn session_picker_selection_highlights_without_eating_the_badge() {
        // selected = 0, i.e. the connecting row.
        let mut app = app_with_picker(crate::app::SessionPickerPurpose::SwitchSession, "");
        wear(
            &mut app,
            "[components.picker]\n\
             row_selected = { foreground = \"#ff5001\", background = \"#005001\" }\n\
             badge_warning = \"#ff4002\"\n",
        );
        let buf = render_to_buffer(&app, 80, 24);

        let (sel_x, sel_y) = picker_row(&buf, "conn");
        let (other_x, other_y) = picker_row(&buf, "exit");

        let bar = Color::Rgb(0x00, 0x50, 0x01);
        assert_eq!(buf[(sel_x, sel_y)].bg, bar, "selected row");
        assert_ne!(buf[(other_x, other_y)].bg, bar, "unselected row");
        // The badge style is foreground-only, so the bar underneath survives it.
        assert_eq!(
            buf[(sel_x, sel_y)].fg,
            Color::Rgb(0xff, 0x40, 0x02),
            "the highlight must not swallow the lifecycle colour"
        );
    }

    /// A perimeter ring for a `Paint` role, as a `[gradients.*]` + role pair.
    ///
    /// Three stops with the first and last equal, because a `perimeter`
    /// gradient closes on itself and validation rejects a visible seam.
    pub(crate) fn ring_gradient(role_table: &str, role_key: &str) -> String {
        format!(
            "[gradients.ring]\ndirection = \"perimeter\"\n\
             stops = [ {{ at = 0.0, color = \"#ff0000\" }}, \
             {{ at = 0.5, color = \"#0000ff\" }}, \
             {{ at = 1.0, color = \"#ff0000\" }} ]\n\
             [{role_table}]\n{role_key} = {{ gradient = \"gradients.ring\" }}\n"
        )
    }

    /// `components.picker.border` is the session picker's own frame role, so a
    /// gradient on it has to reach the frame — the popup contract applies to
    /// the picker's role just as much as to `popup.border`.
    #[test]
    fn a_gradient_picker_border_reaches_the_session_picker_frame() {
        let mut app = app_with_picker(crate::app::SessionPickerPurpose::NewSession, "zzzznope");
        app.session_picker.as_mut().unwrap().return_mode = AppMode::Normal;
        wear(&mut app, &ring_gradient("components.picker", "border"));

        let buf = render_to_buffer(&app, 80, 24);
        let popup = app.last_popup_rect.get().expect("the picker drew");
        let bottom: Vec<_> = (popup.x..popup.right())
            .map(|x| buf[(x, popup.bottom() - 1)].fg)
            .collect();
        assert!(
            bottom.windows(2).any(|pair| pair[0] != pair[1]),
            "the picker border stayed flat: {bottom:?}"
        );
    }

    /// All three picker purposes keep their title, their empty state and the
    /// popup roles that frame them — under markers, not under `default`.
    #[test]
    fn session_picker_purposes_keep_their_chrome_under_a_theme() {
        use crate::app::SessionPickerPurpose::{NewSession, SftpLeftPane, SwitchSession};

        const MARKERS: &str = "[components.popup]\n\
             border = \"#ff6005\"\n\
             title = { foreground = \"#ff6002\" }\n\
             legend = { foreground = \"#ff6003\" }\n\
             [components.picker]\n\
             border = \"#ff6001\"\n\
             query = { foreground = \"#ff6004\" }\n";

        for (purpose, title, empty) in [
            (NewSession, "new session tab", "(no matching hosts)"),
            (SftpLeftPane, "select left server", "(no matching hosts)"),
            (SwitchSession, "switch session", "(no matching sessions)"),
        ] {
            let mut app = app_with_picker(purpose, "zzzznope");
            app.session_picker.as_mut().unwrap().return_mode = AppMode::Normal;
            wear(&mut app, MARKERS);
            let buf = render_to_buffer(&app, 80, 24);
            let popup = app.last_popup_rect.get().expect("the picker drew");

            assert_eq!(
                buf[(popup.x, popup.y)].fg,
                Color::Rgb(0xff, 0x60, 0x01),
                "{purpose:?}: the frame is components.picker.border, the accent \
                 frame the picker has always worn"
            );
            assert_ne!(
                buf[(popup.x, popup.y)].fg,
                Color::Rgb(0xff, 0x60, 0x05),
                "{purpose:?}: not the muted generic popup border"
            );
            assert_eq!(
                style_at_text_in(&buf, popup, title).fg,
                Some(Color::Rgb(0xff, 0x60, 0x02)),
                "{purpose:?}: the title is components.popup.title"
            );
            assert_eq!(
                style_at_text_in(&buf, popup, empty).fg,
                Some(Color::Rgb(0xff, 0x60, 0x03)),
                "{purpose:?}: the empty state is components.popup.legend"
            );
            assert_eq!(
                style_at_text_in(&buf, popup, "zzzznope").fg,
                Some(Color::Rgb(0xff, 0x60, 0x04)),
                "{purpose:?}: the query is components.picker.query"
            );
        }
    }

    #[test]
    fn session_picker_draws_dashboard_or_session_behind_it() {
        let mut app = app_with_picker(crate::app::SessionPickerPurpose::SwitchSession, "");

        // The dashboard's own keybind footer stays under the popup. Read a hint
        // from its left edge, which survives the clipping at 80 columns, rather
        // than one that only fits on a wide terminal.
        app.session_picker.as_mut().unwrap().return_mode = AppMode::Normal;
        let dashboard = buffer_text(&render_to_buffer(&app, 80, 24));
        assert!(
            dashboard.contains("↵ connect"),
            "dashboard keybind footer behind the popup"
        );

        // Both session-ish origins take over the whole frame. Assert positively
        // on the session footer's own clock line, which reads "session M:SS":
        // a negative assert alone would also pass on an empty background, the
        // header hints get clipped at 80 columns with three tabs open, and a
        // bare "session " would match the popup title " switch session ".
        for origin in [AppMode::Session, AppMode::Connecting] {
            app.session_picker.as_mut().unwrap().return_mode = origin;
            let text = buffer_text(&render_to_buffer(&app, 80, 24));
            assert!(
                text.contains("session 0:"),
                "{origin:?}: session footer clock behind the popup"
            );
            assert!(
                !text.contains("↵ connect"),
                "{origin:?}: dashboard keybind footer must be gone"
            );
        }
    }

    #[test]
    fn session_picker_survives_narrow_and_wide_glyphs() {
        use crate::session::{SessionConfig, SessionMeta};

        for w in [1u16, 10, 12, 15, 20, 24, 33, 40, 56, 80] {
            for h in [1u16, 3, 8, 14, 24] {
                let app = app_with_picker(crate::app::SessionPickerPurpose::SwitchSession, "");
                let _ = render_to_buffer(&app, w, h);
            }
        }

        // The endpoint must begin after the *terminal-cell width* of the name,
        // not its scalar count. These three cases independently pin CJK,
        // emoji, and combining-mark advancement.
        for (name, expected_offset) in [("日本語の", 21u16), ("🚀", 15u16), ("e\u{0301}dge", 17u16)]
        {
            let mut app = test_app_with_hosts();
            let cfg = SessionConfig {
                argv: vec!["true".into()],
                display_name: name.into(),
                meta: SessionMeta {
                    address: Some("10.0.0.1".into()),
                    port: Some(22),
                    ..Default::default()
                },
                pending_secret: None,
                key_push_identity: None,
                host_name: name.into(),
            };
            app.sessions
                .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
            app.active_session = Some(0);
            app.session_picker = Some(crate::app::SessionPicker {
                purpose: crate::app::SessionPickerPurpose::SwitchSession,
                query: String::new(),
                selected: 0,
                return_mode: AppMode::Normal,
            });
            app.mode = AppMode::SessionPicker;

            let buf = render_to_buffer(&app, 80, 14);
            let (x, y) = picker_row(&buf, "conn");
            assert_eq!(
                cells_at(&buf, x + expected_offset, y, 10),
                "10.0.0.1:2",
                "{name:?}: endpoint column"
            );
        }
    }

    #[test]
    fn dashboard_footer_shows_keybinds() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 132, 38);
        assert!(buffer_contains(&buffer, "connect"));
        assert!(buffer_contains(&buffer, "quit"));
    }

    #[test]
    fn palette_popup_interior_filled_with_theme_bg() {
        // Regression: the palette overlay used to leave its interior at the
        // terminal default background while the group/user columns were painted
        // theme::BG, producing dark vertical bars. The whole interior must now
        // be theme::BG (or SEL_BG on the selected row).
        //
        // The original bars were default-background holes between painted
        // columns. Since SSHub ships opaque, such a hole can no longer appear
        // by omission — every unclaimed cell gets the canvas — so what this
        // guards now is that nothing *re-introduces* one, and that the columns
        // still agree with the interior around them.
        let mut app = test_app_with_many_hosts(92);
        app.mode = AppMode::Palette;
        app.palette_results = (0..92).collect();
        app.palette_selected = 0;
        let buf = render_to_buffer(&app, 120, 38);

        let holes: Vec<_> = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| buf.cell((x, y)).unwrap().bg == Color::Reset)
            .collect();
        assert!(
            holes.is_empty(),
            "{} default-bg holes, starting at {:?}",
            holes.len(),
            holes.first()
        );

        // The body rows really are the popup's: theme::BG interior, SEL_BG on
        // the selected row, and nothing else wide enough to be a column bar.
        let body_rows = (0..buf.area.height)
            .filter(|&y| {
                (0..buf.area.width)
                    .any(|x| buf.cell((x, y)).unwrap().bg == Color::Rgb(0x0b, 0x0d, 0x10))
            })
            .count();
        assert!(body_rows > 10, "expected to inspect the popup body rows");
    }

    #[test]
    fn render_palette_mode_shows_query() {
        let mut app = test_app_with_hosts();
        app.mode = AppMode::Palette;
        app.palette_query = "web".into();
        app.palette_results = vec![0];
        app.palette_selected = 0;
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "web"));
        assert!(buffer_contains(&buffer, "quick connect"));
    }

    #[test]
    fn render_dashboard_shows_header_stats() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "hosts:"));
        assert!(buffer_contains(&buffer, "online"));
    }

    #[test]
    fn header_stats_count_unreachable_hosts() {
        use crate::ping::{classify_ping, PingClass, PING_UNREACHABLE};

        let mut app = test_app_with_many_hosts(3);
        app.ping_data.insert("host-00".into(), vec![50]);
        app.ping_data.insert("host-01".into(), vec![120]);
        app.ping_data
            .insert("host-02".into(), vec![PING_UNREACHABLE]);

        let [total, online, slow, down] = compute_header_stats(&app);
        assert_eq!(total, 3);
        assert_eq!(online, 1);
        assert_eq!(slow, 1);
        assert_eq!(down, 1);
        assert_eq!(
            classify_ping(app.ping_data.get("host-02").map(|v| v.as_slice())),
            PingClass::Unreachable
        );
    }

    #[test]
    fn render_hides_detail_panel_when_disabled() {
        let mut app = test_app_with_hosts();
        app.config.appearance.show_detail_panel = false;
        let buffer = render_to_buffer(&app, 120, 38);
        // Host name should still be visible in hosts panel
        assert!(buffer_contains(&buffer, "web-prod"));
    }

    #[test]
    fn render_host_list_shows_favorite_star() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 120, 38);
        // The hosts panel shows host name; favorites are indicated by the panel
        assert!(buffer_contains(&buffer, "web-prod"));
    }

    fn test_app_with_many_hosts(n: usize) -> App {
        let mut app = test_app_with_hosts();
        app.hosts = (0..n)
            .map(|i| {
                let name = format!("host-{i:02}");
                let mut h = SshHost::new(&name);
                h.hostname = Some(format!("10.0.0.{i}"));
                HostEntry::Legacy {
                    host: h,
                    meta: HostMetadata {
                        host_name: name,
                        ..Default::default()
                    },
                }
            })
            .collect();
        app.filtered_indices = (0..n).collect();
        app.selected = 0;
        app.rebuild_filter();
        app
    }

    #[test]
    fn group_manage_renders_as_themed_popup() {
        use crate::store::NewHostGroup;
        let store = test_store();
        store
            .create_group(&NewHostGroup {
                name: "prod".into(),
                sort_order: 0,
                ..Default::default()
            })
            .unwrap();

        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(EmptyResolver),
                metadata: Arc::new(MetadataDb::default()),
                store,
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();
        app.mode = AppMode::GroupManage;

        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "Groups"), "popup title missing");
        assert!(buffer_contains(&buffer, "prod"), "group row missing");
        assert!(buffer_contains(&buffer, "a add"), "action hint missing");
        // The scrapped legacy layout had a left "Hosts"/"Groups" sidebar list.
        assert!(
            !buffer_contains(&buffer, "  Hosts"),
            "legacy sidebar should be gone"
        );
    }

    #[test]
    fn nested_group_renders_indented() {
        use crate::store::{NewHost, NewHostGroup};
        let store = test_store();
        let parent = store
            .create_group(&NewHostGroup {
                name: "prod".into(),
                sort_order: 0,
                ..Default::default()
            })
            .unwrap();
        let child = store
            .create_group(&NewHostGroup {
                name: "europe".into(),
                sort_order: 1,
                parent_id: Some(parent.id),
                ..Default::default()
            })
            .unwrap();
        store
            .create_host(&NewHost {
                name: "p1".into(),
                address: "10.0.0.1".into(),
                port: 22,
                group_id: Some(parent.id),
                ..Default::default()
            })
            .unwrap();
        store
            .create_host(&NewHost {
                name: "e1".into(),
                address: "10.0.0.2".into(),
                port: 22,
                group_id: Some(child.id),
                ..Default::default()
            })
            .unwrap();

        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(EmptyResolver),
                metadata: Arc::new(MetadataDb::default()),
                store,
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();

        let buffer = render_to_buffer(&app, 120, 38);
        // Both headers render; the child sits indented under the parent.
        let indent = |needle: &str| -> Option<usize> {
            for y in 0..buffer.area.height {
                let line: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                if let Some(pos) = line.find(needle) {
                    return Some(pos);
                }
            }
            None
        };
        let parent_col = indent("prod").expect("parent header rendered");
        let child_col = indent("europe").expect("child header rendered");
        assert!(
            child_col > parent_col,
            "child group should be indented deeper than its parent ({child_col} > {parent_col})"
        );
    }

    #[test]
    fn failed_connect_shows_x_and_reason() {
        let mut app = test_app_with_hosts();
        let config = crate::session::SessionConfig {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf 'ssh: connect to host h port 22: Connection refused' 1>&2; exit 1".into(),
            ],
            display_name: "web-prod".into(),
            meta: crate::session::SessionMeta {
                address: Some("10.0.0.1".into()),
                ..Default::default()
            },
            pending_secret: None,
            key_push_identity: None,
            host_name: "web-prod".into(),
        };
        let session = crate::session::Session::spawn(config, 24, 80, None).unwrap();
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;

        // Drive the session to exit and flush its stderr.
        for _ in 0..200 {
            app.sessions[0].drain();
            let s = &app.sessions[0];
            let exited = matches!(s.phase, crate::session::SessionPhase::Exited { .. });
            if exited && s.debug_log().to_ascii_lowercase().contains("refused") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "\u{2717}"), "failure X missing");
        assert!(buffer_contains(&buffer, "couldn't connect to"));
        assert!(
            buffer_contains(&buffer, "nothing is listening"),
            "plain-language reason missing"
        );
    }

    #[test]
    fn connecting_screen_shows_spinner_overlay() {
        let mut app = test_app_with_hosts();
        let config = crate::session::SessionConfig {
            argv: vec!["sleep".into(), "1".into()],
            display_name: "web-prod".into(),
            meta: crate::session::SessionMeta {
                address: Some("10.0.0.1".into()),
                ..Default::default()
            },
            pending_secret: None,
            key_push_identity: None,
            host_name: "web-prod".into(),
        };
        let session = crate::session::Session::spawn(config, 24, 80, None).unwrap();
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;
        let buffer = render_to_buffer(&app, 120, 38);
        // The connect overlay replaces the raw PTY dump with a spinner + hint.
        assert!(buffer_contains(&buffer, "connecting to"));
        assert!(buffer_contains(&buffer, "expand log"));
    }

    #[test]
    fn dashboard_shows_open_session_strip() {
        let mut app = test_app_with_hosts();
        let config = crate::session::SessionConfig {
            argv: vec!["true".into()],
            display_name: "web-prod".into(),
            meta: crate::session::SessionMeta::default(),
            pending_secret: None,
            key_push_identity: None,
            host_name: "web-prod".into(),
        };
        let session = crate::session::Session::spawn(config, 24, 80, None).unwrap();
        app.sessions.push(session);
        app.active_session = Some(0);
        // Stays on the dashboard (Normal), so the strip is what makes the
        // background session visible.
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "open"));
        // Host name appears both in the list and in the strip; the strip marker
        // (●) must be present on the top row.
        let top: String = (0..120).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(top.contains('\u{25cf}'), "session dot missing on top row");
        assert!(top.contains("web-prod"), "session name missing on top row");
    }

    #[test]
    fn keys_tab_scrolls_to_keep_selection_visible() {
        use crate::store::Identity;

        let mut app = test_app_with_hosts();
        app.active_tab = 3;
        app.identities = (0..30)
            .map(|i| Identity {
                id: i as i64,
                name: format!("key-{i:02}"),
                username: None,
                private_key: None,
                certificate: None,
                has_password: false,
            })
            .collect();

        app.identity_selected = 0;
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "key-00"));

        // The grid scrolls to the selection over a few frames now (#35), so
        // run it out with a backdated frame clock before looking.
        app.identity_selected = 28;
        let mut buffer = render_to_buffer(&app, 120, 38);
        for _ in 0..40 {
            app.keys_scroll_at.set(Some(
                std::time::Instant::now() - std::time::Duration::from_millis(16),
            ));
            buffer = render_to_buffer(&app, 120, 38);
        }
        assert!(
            buffer_contains(&buffer, "key-28"),
            "selected key card scrolled off-screen"
        );
        assert!(
            !buffer_contains(&buffer, "key-00"),
            "keys grid did not scroll; first card still visible"
        );
    }

    #[test]
    fn hosts_panel_scrolls_to_keep_selection_visible() {
        let mut app = test_app_with_many_hosts(60);

        // Selection at the top: first host visible.
        app.selected = 0;
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "host-00"));

        // Selecting a host far down must bring it into view (it would be off
        // the bottom of the panel without scrolling).
        app.selected = 58;
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(
            buffer_contains(&buffer, "host-58"),
            "selected host scrolled off-screen"
        );
        // And the top of the list should have scrolled away.
        assert!(
            !buffer_contains(&buffer, "host-00"),
            "list did not scroll; top host still visible"
        );
    }

    #[test]
    fn help_overlay_shows_query_and_filters() {
        let mut app = test_app_with_hosts();
        app.mode = AppMode::Help;
        let full = render_to_buffer(&app, 100, 40);
        assert!(buffer_contains(&full, "navigate"));
        assert!(buffer_contains(&full, "type to filter"));
        assert!(buffer_contains(&full, "›"));

        app.help_query = "favorite".into();
        let filtered = render_to_buffer(&app, 100, 40);
        assert!(buffer_contains(&filtered, "Toggle favorite"));
        assert!(buffer_contains(&filtered, "hosts (tab 1)"));
        assert!(!buffer_contains(&filtered, "Cycle filter"));
    }

    #[test]
    fn keybind_editor_shows_query_and_filters() {
        let mut app = test_app_with_hosts();
        app.keybind_editor = Some(crate::app::KeybindEditor {
            selected: 0,
            scroll: 0,
            capturing: false,
            append: false,
            query: "quit".into(),
        });
        app.mode = AppMode::KeybindEditor;
        let buffer = render_to_buffer(&app, 100, 40);
        assert!(buffer_contains(&buffer, "› quit"));
        assert!(buffer_contains(&buffer, "Quit"));
        assert!(buffer_contains(&buffer, "type to filter"));
        assert!(!buffer_contains(&buffer, "Save form"));
    }

    /// Every overlay list highlights its selected row through the role the
    /// catalogue publishes for *that* list. Each family gets its own marker, so
    /// a screen reaching for a neighbour's role fails on an exact colour rather
    /// than looking plausible under `default`.
    #[test]
    fn overlay_selected_rows_use_their_own_row_roles() {
        const MARKERS: &str = "[components.command_palette]\n\
             row_selected = { foreground = \"#ff1001\", background = \"#001001\" }\n\
             [components.settings]\n\
             row_selected = { foreground = \"#ff1002\", background = \"#001002\" }\n\
             [components.keybind]\n\
             row_selected = { foreground = \"#ff1003\", background = \"#001003\" }\n\
             row = { foreground = \"#ff1004\" }\n";

        // Command palette: the selected host name, and a second row that must
        // not be highlighted.
        let mut app = test_app_with_many_hosts(6);
        app.mode = AppMode::Palette;
        app.palette_query.clear();
        app.palette_results = (0..app.hosts.len()).collect();
        app.palette_selected = 0;
        wear(&mut app, MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = app.last_popup_rect.get().expect("the palette drew");
        assert_eq!(
            style_at_text_in(&buf, popup, "host-00").bg,
            Some(ratatui::style::Color::Rgb(0x00, 0x10, 0x01)),
            "the palette highlights with components.command_palette.row_selected"
        );
        assert_ne!(
            style_at_text_in(&buf, popup, "host-01").bg,
            Some(ratatui::style::Color::Rgb(0x00, 0x10, 0x01)),
            "an unselected palette row keeps the popup ground"
        );

        // Settings: row 1 selected, row 0 not.
        let mut app = test_app_with_hosts();
        app.mode = AppMode::Settings;
        app.settings_selected = 1;
        wear(&mut app, MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let selected = crate::app::SETTINGS_ITEMS[1].label;
        let other = crate::app::SETTINGS_ITEMS[0].label;
        assert_eq!(
            style_at_text(&buf, selected).bg,
            Some(ratatui::style::Color::Rgb(0x00, 0x10, 0x02)),
            "settings highlight with components.settings.row_selected"
        );
        assert_ne!(
            style_at_text(&buf, other).bg,
            Some(ratatui::style::Color::Rgb(0x00, 0x10, 0x02)),
            "an unselected settings row is not highlighted"
        );

        // Keybind editor: both roles of the pair, in one render.
        let mut app = test_app_with_hosts();
        app.mode = AppMode::KeybindEditor;
        app.keybind_editor = Some(crate::app::KeybindEditor {
            selected: 0,
            scroll: 0,
            capturing: false,
            append: false,
            query: String::new(),
        });
        wear(&mut app, MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let first = crate::config::KeyAction::ALL[0].label();
        let second = crate::config::KeyAction::ALL[1].label();
        assert_eq!(
            style_at_text(&buf, first).bg,
            Some(ratatui::style::Color::Rgb(0x00, 0x10, 0x03)),
            "the highlighted binding wears components.keybind.row_selected"
        );
        assert_eq!(
            style_at_text(&buf, second).fg,
            Some(ratatui::style::Color::Rgb(0xff, 0x10, 0x04)),
            "every other binding wears components.keybind.row"
        );
    }

    /// The keybind list keeps `text_highlight` where the tunnels tab keeps
    /// `selection_fg`; the catalogue diverges here on purpose.
    #[test]
    fn keybind_and_tunnels_selections_stay_distinct() {
        let theme = crate::test_support::resolved_default();
        assert_eq!(
            theme.style(StyleRole::KeybindRowSelected).fg,
            Some(theme.semantic().text_highlight)
        );
        assert_eq!(
            theme.style(StyleRole::TunnelsRowSelected).fg,
            Some(theme.semantic().selection_fg)
        );
        assert_ne!(
            theme.style(StyleRole::KeybindRowSelected),
            theme.style(StyleRole::TunnelsRowSelected)
        );
    }

    /// The active-field marker is `components.focus.indicator` and nothing
    /// else. Both halves of the label pair are rendered, so neither the focused
    /// nor the editing role can pass by never being drawn.
    #[test]
    fn form_labels_and_focus_indicator_are_independently_themed() {
        const MARKERS: &str = "[components.focus]\n\
             indicator = { foreground = \"#ff2001\" }\n\
             [components.group_form]\n\
label = { foreground = \"#ab0001\" }\n\
label_focused = { foreground = \"#ab0002\" }\n\
value = { foreground = \"#ab0003\" }\n\
value_focused = { foreground = \"#ab0004\" }\n\
marker = { foreground = \"#ab0005\" }\n\
[components.form]\n\
             label = { foreground = \"#ff2002\" }\n\
             label_focused = { foreground = \"#ff2003\" }\n\
             label_editing = { foreground = \"#ff2004\" }\n\
             input_editing = { foreground = \"#ff2005\" }\n";
        let focus = Some(ratatui::style::Color::Rgb(0xff, 0x20, 0x01));

        for (editing, marker, label_fg) in [
            (false, "> ", ratatui::style::Color::Rgb(0xff, 0x20, 0x03)),
            (
                true,
                "\u{25b8} ",
                ratatui::style::Color::Rgb(0xff, 0x20, 0x04),
            ),
        ] {
            let mut app = test_app_with_hosts();
            app.enter_host_form(None, false).unwrap();
            let form = app.host_form.as_mut().unwrap();
            form.field = crate::app::HostFormField::Address;
            form.editing = editing;
            wear(&mut app, MARKERS);
            let buf = render_to_buffer(&app, 120, 38);

            let (mx, my) = crate::test_support::find_text(&buf, &format!("{marker}Address"));
            assert_eq!(
                Some(buf[(mx, my)].fg),
                focus,
                "editing={editing}: the marker is the focus indicator"
            );
            assert_eq!(
                style_at_text(&buf, "Address:").fg,
                Some(label_fg),
                "editing={editing}: the label is themed apart from the marker"
            );
            // A field that is neither focused nor editing keeps the plain role.
            assert_eq!(
                style_at_text(&buf, "Port:").fg,
                Some(ratatui::style::Color::Rgb(0xff, 0x20, 0x02)),
                "editing={editing}: an idle label wears components.form.label"
            );
        }
    }

    /// Popup chrome, on a generic overlay rather than only on the palette.
    ///
    /// `components.popup.background` used to be reachable from exactly one
    /// screen: every other overlay cleared its rect and drew straight onto the
    /// reset ground, so a theme could publish a popup background that no popup
    /// wore. App and popup grounds carry different markers here, so the two
    /// cannot be confused for one another.
    #[test]
    fn a_generic_popup_wears_the_popup_background_not_the_app_background() {
        let mut app = test_app_with_hosts();
        app.mode = AppMode::Help;
        wear(
            &mut app,
            &format!("{OVERLAY_MARKERS}[components.app]\nbackground = \"#123456\"\n"),
        );
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);

        // A cell inside the frame that the help body leaves blank: the last
        // inner column of the first body row.
        let inside = (popup.right() - 2, popup.y + 1);
        assert_eq!(
            buf[inside].bg,
            marker(0x0a0b0c),
            "the help popup interior is components.popup.background"
        );
        assert_ne!(
            buf[inside].bg,
            marker(0x123456),
            "the app background must not stand in for the popup's own"
        );
        // Outside the popup the app background still shows through.
        assert_eq!(
            buf[(0, 0)].bg,
            marker(0x123456),
            "the frame around the popup keeps components.app.background"
        );

        // ...and the palette, which has always been opaque, keeps its own fill.
        let mut app = test_app_with_many_hosts(6);
        app.mode = AppMode::Palette;
        app.palette_results = (0..app.hosts.len()).collect();
        app.palette_selected = 0;
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);
        assert_eq!(
            buf[(popup.right() - 2, popup.y + 1)].bg,
            marker(0x0a0b0c),
            "the palette honours a theme's own popup background"
        );
    }

    /// The four cells an isolated legacy golden caught drifting: the help
    /// section heading, the session-picker frame, the group form's focused
    /// field and the palette query. Read under `default`, through the real
    /// renderer, against the exact `theme.rs` value each one replaced.
    #[test]
    fn the_overlays_reproduce_their_legacy_cells_under_default() {
        use crate::tui::theme::legacy;

        // 1. Help section heading — `theme::heading()`, bright *and* bold.
        let mut app = test_app_with_hosts();
        app.mode = AppMode::Help;
        let buf = render_to_buffer(&app, 120, 38);
        let heading = style_at_text(&buf, "navigate");
        assert_eq!(heading.fg, Some(legacy::BRIGHT), "help section colour");
        assert!(
            heading.add_modifier.contains(Modifier::BOLD),
            "help section weight"
        );

        // 2. Session-picker frame — the accent, not the muted popup border.
        let mut app = app_with_picker(crate::app::SessionPickerPurpose::SwitchSession, "");
        app.session_picker.as_mut().unwrap().return_mode = AppMode::Normal;
        let buf = render_to_buffer(&app, 80, 24);
        let popup = drawn_popup(&app);
        assert_eq!(
            buf[(popup.x, popup.y)].fg,
            legacy::ACCENT,
            "the session picker was framed in the accent"
        );
        assert_ne!(
            buf[(popup.x, popup.y)].fg,
            legacy::MUTE,
            "not the popup border"
        );

        // 3. Group form — the focused label and value were both bright + bold.
        let mut app = test_app_with_hosts();
        app.group_form = Some(crate::app::GroupFormEdit {
            id: None,
            name: "alpha".into(),
            cursor: 0,
            field: crate::app::GroupFormField::Name,
            default_identity_id: None,
            parent_id: None,
            return_to_manage: false,
        });
        app.mode = AppMode::GroupForm;
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);
        let label = style_at_text_in(&buf, popup, "Name:");
        assert_eq!(label.fg, Some(legacy::BRIGHT), "focused group label colour");
        assert!(
            label.add_modifier.contains(Modifier::BOLD),
            "focused group label weight"
        );
        let value = style_at_text_in(&buf, popup, "alpha");
        assert_eq!(value.fg, Some(legacy::BRIGHT), "focused group value colour");
        assert!(
            value.add_modifier.contains(Modifier::BOLD),
            "focused group value weight"
        );
        // The unfocused rows kept `mute()` / `text()`.
        assert_eq!(
            style_at_text_in(&buf, popup, "Parent group:").fg,
            Some(legacy::MUTE),
            "an idle group label"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "(top level)").fg,
            Some(legacy::TEXT),
            "an idle group value"
        );

        // 4. Palette query — `white()`, a shade above the picker's `bright()`.
        let mut app = test_app_with_many_hosts(6);
        app.mode = AppMode::Palette;
        app.palette_query = "zzq".into();
        app.palette_results = vec![];
        app.palette_selected = 0;
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);
        assert_eq!(
            style_at_text_in(&buf, popup, "zzq").fg,
            Some(legacy::WHITE),
            "the palette typed in white"
        );
        assert_ne!(
            style_at_text_in(&buf, popup, "zzq").fg,
            Some(legacy::BRIGHT),
            "not the session picker's query colour"
        );
    }

    /// The five row/field markers that used to be glued to a styled label and
    /// therefore wore that label's own `theme.rs` cell.
    ///
    /// Splitting the marker into its own span made it independently themeable,
    /// which was the point — but under `default` it must still land on the
    /// colour it always had, and for four of the five families that is *not*
    /// the accent `components.focus.indicator` resolves to.
    #[test]
    fn the_field_markers_reproduce_their_legacy_cells_under_default() {
        use crate::tui::theme::legacy;

        // 1 + 2. Both picker overlays drew the marker in `theme::selected()`.
        let mut app = test_app_with_hosts();
        app.mode = AppMode::TagFilter;
        app.tag_filter_selected = 0;
        let buf = render_to_buffer(&app, 120, 38);
        let (ax, ay) = crate::test_support::find_text(&buf, "(all)");
        assert_eq!(
            cell_style(&buf, ax - 2, ay),
            legacy::selected(),
            "the tag filter's marker was the selected row's own style"
        );

        let mut app = test_app_with_hosts();
        app.enter_host_form(None, false).unwrap();
        app.groups = vec![crate::store::HostGroup {
            id: 1,
            name: "alpha".into(),
            sort_order: 0,
            default_identity_id: None,
            parent_id: None,
            reserved: false,
        }];
        app.field_picker = Some(crate::app::FieldPicker {
            kind: crate::app::PickerKind::Group,
            selected: 0,
            creating: None,
            cursor: 0,
        });
        app.mode = AppMode::FieldPicker;
        let buf = render_to_buffer(&app, 120, 38);
        let (gx, gy) = crate::test_support::find_text(&buf, "[ ] alpha");
        assert_eq!(
            cell_style(&buf, gx - 2, gy),
            legacy::selected(),
            "the field picker's marker likewise"
        );

        // 3. The group form's marker was glued to `theme::heading()`.
        let mut app = test_app_with_hosts();
        app.group_form = Some(crate::app::GroupFormEdit {
            id: None,
            name: "alpha".into(),
            cursor: 0,
            field: crate::app::GroupFormField::Name,
            default_identity_id: None,
            parent_id: None,
            return_to_manage: false,
        });
        app.mode = AppMode::GroupForm;
        let buf = render_to_buffer(&app, 120, 38);
        let (mx, my) = crate::test_support::find_text(&buf, "\u{25b8} Name");
        let marker = cell_style(&buf, mx, my);
        assert_eq!(marker.fg, Some(legacy::BRIGHT), "group form marker colour");
        assert!(
            marker.add_modifier.contains(Modifier::BOLD),
            "group form marker weight"
        );
        assert_ne!(
            marker.fg,
            Some(legacy::ACCENT),
            "not the global focus indicator's accent"
        );

        // 4 + 5. Both settings-shaped popups drew it in `white()` on the bar.
        let mut app = test_app_with_hosts();
        app.keybind_editor = Some(crate::app::KeybindEditor {
            selected: 0,
            scroll: 0,
            capturing: false,
            append: false,
            query: String::new(),
        });
        app.mode = AppMode::KeybindEditor;
        let buf = render_to_buffer(&app, 120, 38);
        let (kx, ky) =
            crate::test_support::find_text(&buf, crate::config::KeyAction::ALL[0].label());
        assert_eq!(
            cell_style(&buf, kx - 2, ky),
            legacy::white().bg(legacy::SEL_BG),
            "the keybind editor's marker rode the highlighted row"
        );

        let mut app = test_app_with_hosts();
        app.mode = AppMode::TunnelReconnectSettings;
        app.tunnel_reconnect_selected = 0;
        let buf = render_to_buffer(&app, 120, 38);
        let label = crate::app::TUNNEL_RECONNECT_FIELDS[0].0;
        let (tx, ty) = crate::test_support::find_text(&buf, label);
        assert_eq!(
            cell_style(&buf, tx - 2, ty),
            legacy::white().bg(legacy::SEL_BG),
            "the tunnel-reconnect marker likewise"
        );

        // The two forms whose marker only ever had a direct ANSI colour keep
        // the global role, and that role is the accent. Both are rendered:
        // documenting a two-form exception and proving one of them is how a
        // half-bound role passes for a bound one.
        let mut app = test_app_with_hosts();
        app.enter_host_form(None, false).unwrap();
        app.host_form.as_mut().unwrap().editing = false;
        let buf = render_to_buffer(&app, 120, 38);
        let (hx, hy) = crate::test_support::find_text(&buf, "> Address");
        assert_eq!(
            cell_style(&buf, hx, hy).fg,
            Some(legacy::ACCENT),
            "the host form's marker is components.focus.indicator"
        );

        let mut app = test_app_with_hosts();
        app.enter_identity_form(None).unwrap();
        app.identity_form.as_mut().unwrap().editing = false;
        let buf = render_to_buffer(&app, 120, 38);
        let (ix, iy) = crate::test_support::find_text(&buf, "> Name");
        assert_eq!(
            cell_style(&buf, ix, iy).fg,
            Some(legacy::ACCENT),
            "the identity form's marker is components.focus.indicator too"
        );
        // And while editing, where the glyph changes but the role does not.
        app.identity_form.as_mut().unwrap().editing = true;
        let buf = render_to_buffer(&app, 120, 38);
        let (ex, ey) = crate::test_support::find_text(&buf, "\u{25b8} Name");
        assert_eq!(
            cell_style(&buf, ex, ey).fg,
            Some(legacy::ACCENT),
            "the identity form's editing marker likewise"
        );
    }

    /// The three form cells that carried **no colour** before the migration and
    /// deliberately carry one now.
    ///
    /// These are not the spec's direct-ANSI exception — an unstyled `Block`
    /// title, an idle `Style::default()` value and a bare `Modifier::DIM` hint
    /// had no colour at all. Each is a recorded deviation, so each is pinned
    /// here against the value it deliberately took, in both forms that draw it.
    #[test]
    fn the_forms_uncoloured_cells_keep_their_documented_roles() {
        use crate::tui::theme::legacy;

        let mut host = test_app_with_hosts();
        host.enter_host_form(None, false).unwrap();
        host.host_form.as_mut().unwrap().editing = false;

        let mut identity = test_app_with_hosts();
        identity.enter_identity_form(None).unwrap();
        identity.identity_form.as_mut().unwrap().editing = false;

        for (which, app, title, idle_label, hint) in [
            ("host form", &host, "New host", "Port:", "Tab/"),
            (
                "identity form",
                &identity,
                "Identity",
                "Username:",
                "type to edit",
            ),
        ] {
            let buf = render_to_buffer(app, 120, 38);
            let popup = drawn_popup(app);

            // Was: an unstyled Block title. Now: `components.popup.title`,
            // like every other overlay's.
            let title_cell = style_at_text_in(&buf, popup, title);
            assert_eq!(title_cell.fg, Some(legacy::BRIGHT), "{which}: title colour");
            assert!(
                title_cell.add_modifier.contains(Modifier::BOLD),
                "{which}: title weight"
            );

            // Was: `Style::default()`. Now: `components.form.value`.
            let (lx, ly) = crate::test_support::find_text(&buf, idle_label);
            assert_eq!(
                cell_style(&buf, lx + idle_label.chars().count() as u16 + 1, ly).fg,
                Some(legacy::TEXT),
                "{which}: an idle value takes the body-text role"
            );

            // Was: the bare DIM modifier. Now: `components.form.help`.
            assert_eq!(
                style_at_text_in(&buf, popup, hint).fg,
                Some(legacy::DIM),
                "{which}: the key hints take the help role"
            );
        }
    }

    /// A popup title keeps the weight it always had under `default`.
    ///
    /// Cell-exact, through the real renderer, including the modifier — the role
    /// parity assertion in `theme::builtins` proves the resolved style, this
    /// proves the cell it reaches.
    #[test]
    fn popup_titles_stay_bold_under_default() {
        let mut app = test_app_with_hosts();
        app.mode = AppMode::Help;
        let buf = render_to_buffer(&app, 120, 38);
        let (x, y) = crate::test_support::find_text(&buf, " Help ");
        let cell = &buf[(x + 1, y)];
        assert_eq!(
            cell.fg,
            crate::tui::theme::legacy::BRIGHT,
            "legacy heading colour"
        );
        assert!(
            cell.modifier.contains(Modifier::BOLD),
            "legacy heading weight"
        );
    }

    /// The help sheet's three roles, all on one rendered frame.
    #[test]
    fn the_help_sheet_wears_its_own_three_roles() {
        let app = marked(AppMode::Help);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);

        assert_eq!(
            style_at_text_in(&buf, popup, "navigate").fg,
            Some(marker(0xa70001)),
            "a section heading is components.help.section"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "Tab ").fg,
            Some(marker(0xa70002)),
            "a key column is components.help.key"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "Toggle detail panel").fg,
            Some(marker(0xa70003)),
            "a description is components.help.description"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "PgUp/PgDn scroll").fg,
            Some(marker(0xa10003)),
            "the fixed footer is components.popup.hint"
        );
    }

    /// Every form role, across the three states a field can be in.
    #[test]
    fn the_host_form_wears_every_form_role() {
        // Idle + focused: the Address field is current but not being edited.
        let mut app = test_app_with_hosts();
        app.enter_host_form(None, false).unwrap();
        app.host_form.as_mut().unwrap().editing = false;
        app.host_notice = Some("nope".into());
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);

        assert_eq!(
            style_at_text_in(&buf, popup, "Port:").fg,
            Some(marker(0xa50001)),
            "an idle label is components.form.label"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "Address:").fg,
            Some(marker(0xa50002)),
            "the current label is components.form.label_focused"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "> ").fg,
            Some(marker(0xa90001)),
            "the marker is components.focus.indicator"
        );
        // The Port row is idle, so its value carries the plain value role.
        let (px, py) = crate::test_support::find_text(&buf, "Port:");
        assert_eq!(
            cell_style(&buf, px + 6, py).fg,
            Some(marker(0xa50004)),
            "an idle value is components.form.value"
        );
        // The focused row's value has its own role.
        let (ax, ay) = crate::test_support::find_text(&buf, "Address:");
        assert_eq!(
            cell_style(&buf, ax + 9, ay).fg,
            Some(marker(0xa50006)),
            "the current value is components.form.input_focused"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "Tab/").fg,
            Some(marker(0xa50008)),
            "the key hints are components.form.help"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "nope").fg,
            Some(marker(0xa50009)),
            "a save failure is components.form.error"
        );

        // Editing: the same field, mid-edit.
        let mut app = test_app_with_hosts();
        app.enter_host_form(None, false).unwrap();
        let form = app.host_form.as_mut().unwrap();
        form.editing = true;
        form.address = "10.0.0.9".into();
        form.cursor = form.address.len();
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);

        assert_eq!(
            style_at_text_in(&buf, popup, "Address:").fg,
            Some(marker(0xa50003)),
            "the edited label is components.form.label_editing"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "10.0.0.9").fg,
            Some(marker(0xa50007)),
            "the edited value is components.form.input_editing"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "\u{25b8} ").fg,
            Some(marker(0xa90001)),
            "the editing marker is still components.focus.indicator"
        );
    }

    /// `components.form.input` — the single-line prompts, which have no
    /// focused/editing distinction to make.
    #[test]
    fn the_prompt_popups_wear_the_plain_input_role() {
        let mut app = test_app_with_hosts();
        app.import_prompt = Some(crate::app::ImportPromptEdit {
            path: "/tmp/termius".into(),
            cursor: 0,
            error: Some("no L00t.csv".into()),
        });
        app.mode = AppMode::ImportPrompt;
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);

        assert_eq!(
            style_at_text_in(&buf, popup, "Path to Termius").fg,
            Some(marker(0xc00001)),
            "the prompt text is components.text.primary"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "/tmp/termius").fg,
            Some(marker(0xa50005)),
            "the typed path is components.form.input"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "no L00t.csv").fg,
            Some(marker(0xa10005)),
            "the prompt error is components.popup.error"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "Esc: cancel").fg,
            Some(marker(0xa10003)),
            "the prompt legend is components.popup.hint"
        );
    }

    /// The group form's own four roles, both field states in one frame.
    #[test]
    fn the_group_form_wears_its_own_focus_roles() {
        let mut app = test_app_with_hosts();
        app.group_form = Some(crate::app::GroupFormEdit {
            id: None,
            name: "alpha".into(),
            cursor: 0,
            field: crate::app::GroupFormField::Name,
            default_identity_id: None,
            parent_id: None,
            return_to_manage: false,
        });
        app.mode = AppMode::GroupForm;
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);

        assert_eq!(
            style_at_text_in(&buf, popup, "Name:").fg,
            Some(marker(0xab0002)),
            "the current label is components.group_form.label_focused"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "alpha").fg,
            Some(marker(0xab0004)),
            "its value is components.group_form.value_focused"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "Parent group:").fg,
            Some(marker(0xab0001)),
            "an idle label is components.group_form.label"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "(top level)").fg,
            Some(marker(0xab0003)),
            "its value is components.group_form.value"
        );
        // The host form's roles must not leak in: they are a different family.
        assert_ne!(
            style_at_text_in(&buf, popup, "Name:").fg,
            Some(marker(0xa50002)),
            "components.form.label_focused belongs to the host form"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "\u{25b8} ").fg,
            Some(marker(0xab0005)),
            "the marker is this family's own components.group_form.marker"
        );
        assert_ne!(
            style_at_text_in(&buf, popup, "\u{25b8} ").fg,
            Some(marker(0xa90001)),
            "not the global focus indicator, which is the host form's"
        );
    }

    /// Both generic table roles, on the group-management popup.
    #[test]
    fn the_group_popup_wears_both_table_roles() {
        let mut app = test_app_with_hosts();
        app.groups = vec![
            crate::store::HostGroup {
                id: 1,
                name: "alpha".into(),
                sort_order: 0,
                default_identity_id: None,
                parent_id: None,
                reserved: false,
            },
            crate::store::HostGroup {
                id: 2,
                name: "bravo".into(),
                sort_order: 1,
                default_identity_id: None,
                parent_id: None,
                reserved: false,
            },
        ];
        app.group_manage_selected = 0;
        app.mode = AppMode::GroupManage;
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);

        assert_eq!(
            style_at_text_in(&buf, popup, "alpha").bg,
            Some(marker(0x060401)),
            "the highlighted group is components.table.row_selected"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "bravo").fg,
            Some(marker(0xa60001)),
            "every other group is components.table.row"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "a add").fg,
            Some(marker(0xa10003)),
            "the action hint is components.popup.hint"
        );
    }

    /// All three keybind value states, plus the row pair, in two frames.
    #[test]
    fn the_keybind_editor_wears_every_keybind_role() {
        for capturing in [false, true] {
            let mut app = test_app_with_hosts();
            app.keybind_editor = Some(crate::app::KeybindEditor {
                selected: 0,
                scroll: 0,
                capturing,
                append: false,
                query: String::new(),
            });
            app.mode = AppMode::KeybindEditor;
            wear(&mut app, OVERLAY_MARKERS);
            let buf = render_to_buffer(&app, 120, 38);
            let popup = drawn_popup(&app);

            let first = crate::config::KeyAction::ALL[0];
            assert_eq!(
                style_at_text_in(&buf, popup, first.label()).bg,
                Some(marker(0x080401)),
                "capturing={capturing}: the current row is keybind.row_selected"
            );
            let (kx, ky) = crate::test_support::find_text(&buf, first.label());
            assert_eq!(
                cell_style(&buf, kx - 2, ky).fg,
                Some(marker(0xa80006)),
                "capturing={capturing}: its marker is components.keybind.marker"
            );
            assert_eq!(
                style_at_text_in(&buf, popup, crate::config::KeyAction::ALL[1].label()).fg,
                Some(marker(0xa80001)),
                "capturing={capturing}: any other row is keybind.row"
            );

            let binds = app.config.keybinds.binds(crate::config::KeyAction::ALL[1]);
            assert_eq!(
                style_at_text_in(&buf, popup, &binds.join(", ")).fg,
                Some(marker(0xa80003)),
                "capturing={capturing}: an idle binding is keybind.value"
            );
            if capturing {
                assert_eq!(
                    style_at_text_in(&buf, popup, "press a key").fg,
                    Some(marker(0xa80005)),
                    "the capture prompt is keybind.value_capturing"
                );
            } else {
                let sel = app.config.keybinds.binds(first);
                assert_eq!(
                    style_at_text_in(&buf, popup, &sel.join(", ")).fg,
                    Some(marker(0xa80004)),
                    "the current binding is keybind.value_bound"
                );
            }
        }
    }

    /// `components.picker.match` and the plain picker row, on the two screens
    /// that own them.
    #[test]
    fn the_settings_and_tag_popups_wear_the_picker_roles() {
        let mut app = test_app_with_hosts();
        app.mode = AppMode::Settings;
        app.settings_selected = 0;
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);
        assert_eq!(
            style_at_text_in(&buf, popup, app.active_theme_id()).fg,
            Some(marker(0xa20002)),
            "the active theme id is components.picker.match"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "pick the active").fg,
            Some(marker(0xa10003)),
            "the row hint is components.popup.hint"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "Enter choose").fg,
            Some(marker(0xa10004)),
            "the key legend is components.popup.legend"
        );

        // The tag filter shows the picker row pair: `(all)` is selected, the
        // real tag below it is not.
        let mut app = test_app_with_hosts();
        app.mode = AppMode::TagFilter;
        app.tag_filter_selected = 0;
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);
        assert_eq!(
            style_at_text_in(&buf, popup, "prod").fg,
            Some(marker(0xa20003)),
            "an unselected tag is components.picker.row"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "(all)").bg,
            Some(marker(0x020401)),
            "the highlighted tag is components.picker.row_selected"
        );
        // Read on the highlighted row itself: the query line one row above also
        // opens with `\u{203a}`, and it is `picker.query`, not the indicator.
        let (ax, ay) = crate::test_support::find_text(&buf, "(all)");
        assert_eq!(
            cell_style(&buf, ax - 2, ay).fg,
            Some(marker(0xa20009)),
            "its marker is components.picker.marker"
        );
    }

    /// The tunnel-reconnect popup, which shares the settings family.
    #[test]
    fn the_tunnel_reconnect_popup_wears_the_settings_roles() {
        let mut app = test_app_with_hosts();
        app.mode = AppMode::TunnelReconnectSettings;
        app.tunnel_reconnect_selected = 0;
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);

        let selected = crate::app::TUNNEL_RECONNECT_FIELDS[0].0;
        let other = crate::app::TUNNEL_RECONNECT_FIELDS[1].0;
        assert_eq!(
            style_at_text_in(&buf, popup, selected).bg,
            Some(marker(0x040401)),
            "the current row is components.settings.row_selected"
        );
        let (sx, sy) = crate::test_support::find_text(&buf, selected);
        assert_eq!(
            cell_style(&buf, sx - 2, sy).fg,
            Some(marker(0xa40002)),
            "its marker is components.settings.marker"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, other).fg,
            Some(marker(0xa60001)),
            "every other row is components.table.row"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "+/- adjust").fg,
            Some(marker(0xa10004)),
            "the legend is components.popup.legend"
        );
    }

    /// The palette's own roles, including the two global ones it reaches for.
    #[test]
    fn the_palette_wears_its_row_and_chrome_roles() {
        let mut app = test_app_with_many_hosts(6);
        app.mode = AppMode::Palette;
        app.palette_query = "zqx".into();
        app.palette_results = (0..app.hosts.len()).collect();
        app.palette_selected = 0;
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 120, 38);
        let popup = drawn_popup(&app);

        assert_eq!(
            style_at_text_in(&buf, popup, "host-00").bg,
            Some(marker(0x030401)),
            "the selected row is components.command_palette.row_selected"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "host-01").fg,
            Some(marker(0xc00002)),
            "an unselected name is components.text.bright"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "zqx").fg,
            Some(marker(0xa30002)),
            "the typed query is components.command_palette.query"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, "\u{276f}").fg,
            Some(marker(0xc10001)),
            "the prompt marker is components.status.success"
        );
        // The rule under the prompt, read at a fixed inner cell — a text search
        // for box-drawing would find the popup frame first.
        assert_eq!(
            buf[(popup.x + 1, popup.y + 2)].fg,
            marker(0xc20001),
            "the rules are components.separator.primary"
        );
        assert_eq!(
            style_at_text_in(&buf, popup, " host ").fg,
            Some(marker(0xa10004)),
            "the detail keys are components.popup.legend"
        );
    }

    /// Every identity-card role, in both card states, plus the agent block.
    #[test]
    fn the_identity_cards_wear_every_identities_role() {
        use crate::ssh::agent::{AgentInfo, AgentKey};

        let mut app = test_app_with_hosts();
        app.active_tab = 3;
        app.identities = vec![
            crate::store::Identity {
                id: 1,
                name: "prod-key".into(),
                username: Some("rootuser".into()),
                private_key: Some(std::path::PathBuf::from("/keys/id_ed25519")),
                certificate: None,
                has_password: true,
            },
            crate::store::Identity {
                id: 2,
                name: "shared-login".into(),
                username: Some("deployer".into()),
                private_key: None,
                certificate: None,
                has_password: true,
            },
        ];
        app.identity_selected = 0;
        app.identity_notice = Some("agent refused".into());
        app.agent_info = Some(AgentInfo {
            socket_path: Some("/run/agent.sock".into()),
            keys: vec![AgentKey {
                bits: "256".into(),
                fingerprint: "SHA256:abcdef".into(),
                comment: "/keys/id_ed25519".into(),
                key_type: "ED25519".into(),
            }],
            forwarding_hosts: 0,
        });
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 132, 38);

        // Card 1 is selected: every role keeps its own foreground and takes
        // only the selection's background.
        let name = style_at_text(&buf, "prod-key");
        assert_eq!(name.fg, Some(marker(0xb10004)), "card.name");
        assert_eq!(name.bg, Some(marker(0x0b0401)), "card.selection backs it");
        assert_eq!(
            style_at_text(&buf, "rootuser").fg,
            Some(marker(0xb10005)),
            "card.text"
        );
        assert_eq!(
            style_at_text(&buf, "SHA256:abc").fg,
            Some(marker(0xb10006)),
            "card.metadata"
        );
        assert_eq!(
            style_at_text(&buf, "ed25519").fg,
            Some(marker(0xb10007)),
            "card.key_type"
        );
        assert_eq!(
            style_at_text(&buf, " loaded").fg,
            Some(marker(0xb10008)),
            "card.loaded"
        );
        assert_eq!(
            style_at_text(&buf, "passphrase").fg,
            Some(marker(0xb1000a)),
            "card.credential"
        );

        // Card 2 is not selected, has no key and is not in the agent.
        assert_eq!(
            style_at_text(&buf, "shared-login").fg,
            Some(marker(0xb10004)),
            "an unselected card keeps card.name"
        );
        assert_ne!(
            style_at_text(&buf, "shared-login").bg,
            Some(marker(0x0b0401)),
            "an unselected card is not backed by the selection"
        );
        assert_eq!(
            style_at_text(&buf, "no key").fg,
            Some(marker(0xb10006)),
            "the keyless note is card.metadata"
        );

        // Card borders, read at each card's top-left corner.
        let (sel_x, sel_y) = crate::test_support::find_text(&buf, "prod-key");
        assert_eq!(
            buf[(sel_x - 2, sel_y - 1)].fg,
            marker(0xb10002),
            "the selected card is framed by card.border_selected"
        );
        let (other_x, other_y) = crate::test_support::find_text(&buf, "shared-login");
        assert_eq!(
            buf[(other_x - 2, other_y - 1)].fg,
            marker(0xb10001),
            "every other card is framed by card.border"
        );

        // The agent block below the grid.
        assert_eq!(
            style_at_text(&buf, "agent socket").fg,
            Some(marker(0xb20002)),
            "agent.label"
        );
        assert_eq!(
            style_at_text(&buf, "/run/agent.sock").fg,
            Some(marker(0xb20003)),
            "agent.value"
        );
        let (cx, cy) = crate::test_support::find_text(&buf, "loaded keys   ");
        assert_eq!(
            cell_style(&buf, cx + 14, cy).fg,
            Some(marker(0xb20004)),
            "agent.count"
        );
        // The rule sits two rows above `loaded keys`: rule, socket, count.
        assert_eq!(buf[(cx, cy - 2)].fg, marker(0xb20001), "agent.separator");
        assert_eq!(
            style_at_text(&buf, "agent refused").fg,
            Some(marker(0xb00002)),
            "identities.notice"
        );
    }

    /// The two identity roles the populated tab cannot show.
    ///
    /// Checked across the terminal widths people actually use: the agent block
    /// used to start at the top of an empty tab and write its own text over the
    /// empty-state row, which at 132 columns happened to land beside the
    /// message and at 80 on top of it.
    #[test]
    fn the_empty_identities_tab_wears_its_own_roles() {
        for width in [20u16, 40, 80, 132] {
            let mut app = test_app_with_hosts();
            app.active_tab = 3;
            app.identities.clear();
            app.agent_info = None;
            wear(&mut app, OVERLAY_MARKERS);
            let buf = render_to_buffer(&app, width, 38);

            let body =
                crate::tui::dashboard_layout::dashboard_layout_zoomed(buf.area, app.ui_zoom).body;
            // `render_keys` draws nothing at all below this size, so there is
            // no cell to read and nothing to claim.
            if body.width < 20 || body.height < 4 {
                continue;
            }
            let inner = crate::tui::screens::keys::inner_width(body.width) as usize;

            // Whichever of the two texts fits is asserted whole; a body too
            // narrow for one still shows its head, and never loses it to the
            // other message landing on the same row.
            for (label, full) in [
                (
                    "empty state",
                    "No identities \u{2014} press 'a' (key or user+password)",
                ),
                ("missing-agent note", "SSH agent not detected"),
            ] {
                let needle: String = if full.chars().count() <= inner {
                    full.to_string()
                } else {
                    full.chars().take(inner.min(13)).collect()
                };
                assert_eq!(
                    style_at_text(&buf, &needle).fg,
                    Some(marker(0xb00001)),
                    "{width} cols: the {label} survives, in identities.empty"
                );
            }
        }
    }

    /// `components.identities.card.missing` — the colour a key that is not in
    /// the agent is drawn in, which the loaded card above cannot show.
    #[test]
    fn an_unloaded_key_card_wears_the_missing_colour() {
        let mut app = test_app_with_hosts();
        app.active_tab = 3;
        app.identities = vec![crate::store::Identity {
            id: 1,
            name: "cold-key".into(),
            username: None,
            private_key: Some(std::path::PathBuf::from("/keys/id_rsa")),
            certificate: None,
            has_password: false,
        }];
        app.agent_info = None;
        wear(&mut app, OVERLAY_MARKERS);
        let buf = render_to_buffer(&app, 132, 38);
        assert_eq!(
            style_at_text(&buf, " not loaded").fg,
            Some(marker(0xb10009)),
            "an absent key is components.identities.card.missing"
        );
    }

    /// Every overlay must clip rather than write outside the buffer when the
    /// terminal is smaller than the popup's own minimum.
    #[test]
    fn tiny_terminals_clip_every_overlay() {
        let modes = [
            AppMode::Palette,
            AppMode::HostForm,
            AppMode::FieldPicker,
            AppMode::IdentityForm,
            AppMode::GroupManage,
            AppMode::GroupForm,
            AppMode::GroupFieldPicker,
            AppMode::TagFilter,
            AppMode::SessionPicker,
            AppMode::Settings,
            AppMode::KeybindEditor,
            AppMode::TunnelReconnectSettings,
            AppMode::ConfirmQuit,
            AppMode::ConfirmDiscard,
            AppMode::ConfirmDelete,
            AppMode::Help,
            AppMode::Notice,
            AppMode::ImportPrompt,
            AppMode::SftpPrompt,
        ];
        for mode in modes {
            for (w, h) in [(1u16, 1u16), (3, 2), (8, 4), (20, 6), (40, 10)] {
                // Every overlay's state is populated, so no mode can pass the
                // matrix by returning early on a `None`.
                let mut app = test_app_with_hosts();
                app.enter_host_form(None, false).unwrap();
                app.enter_identity_form(None).unwrap();
                app.notice_popup = Some("boom".into());
                app.keybind_editor = Some(crate::app::KeybindEditor {
                    selected: 0,
                    scroll: 0,
                    capturing: false,
                    append: false,
                    query: String::new(),
                });
                app.pending_delete = Some(crate::app::PendingDelete::Host {
                    id: 1,
                    name: "web-prod".into(),
                });
                app.field_picker = Some(crate::app::FieldPicker {
                    kind: crate::app::PickerKind::Group,
                    selected: 0,
                    creating: Some("new-group".into()),
                    cursor: 0,
                });
                app.group_form = Some(crate::app::GroupFormEdit {
                    id: None,
                    name: "grp".into(),
                    cursor: 0,
                    field: crate::app::GroupFormField::Name,
                    default_identity_id: None,
                    parent_id: None,
                    return_to_manage: true,
                });
                app.group_field_picker = Some(crate::app::GroupFieldPicker {
                    kind: crate::app::GroupFormField::Parent,
                    selected: 0,
                });
                app.session_picker = Some(crate::app::SessionPicker {
                    purpose: crate::app::SessionPickerPurpose::SwitchSession,
                    query: String::new(),
                    selected: 0,
                    return_mode: AppMode::Normal,
                });
                app.import_prompt = Some(crate::app::ImportPromptEdit {
                    path: "/tmp/termius".into(),
                    cursor: 0,
                    error: Some("no L00t.csv".into()),
                });
                app.sftp_prompt = Some(crate::app::SftpPromptEdit {
                    kind: crate::app::SftpPromptKind::Rename,
                    side: crate::sftp::model::Side::Local,
                    base: std::path::PathBuf::from("/tmp"),
                    old_path: Some(std::path::PathBuf::from("/tmp/notes.txt")),
                    value: "notes.txt".into(),
                    cursor: 0,
                    error: Some("exists".into()),
                });
                app.mode = mode;
                let buf = render_to_buffer(&app, w, h);
                assert_eq!(buf.area.width, w, "{mode:?} at {w}x{h}");
                assert_eq!(buf.area.height, h, "{mode:?} at {w}x{h}");
            }
        }
    }

    // ── Overlay role markers ─────────────────────────────────
    //
    // Default parity can never prove that a role is *read*: an unbound role and
    // a correctly bound one both produce the legacy colour under `default`. So
    // every role this task binds gets a colour no other role uses, is driven
    // through the real production renderer (`render`, full frame), and is read
    // back positively at a named cell with `assert_eq!`. A counter-check
    // against a neighbouring family may be added, but `assert_ne!` never
    // stands in for the positive proof.
    //
    // [`OVERLAY_MARKERS`] below is the single marker theme every one of these
    // tests wears, so no two roles can share a colour by accident.

    /// The one marker theme every overlay proof below wears.
    ///
    /// One unique colour per role this task binds, so a renderer that reaches
    /// for a neighbouring role fails on an exact value instead of looking
    /// plausible. Keep the `#rrggbb` values distinct; a duplicate would make a
    /// wrong binding indistinguishable from a right one.
    pub(crate) const OVERLAY_MARKERS: &str = "\
[components.popup]\n\
background = \"#0a0b0c\"\n\
border = \"#a10001\"\n\
title = { foreground = \"#a10002\" }\n\
hint = { foreground = \"#a10003\" }\n\
legend = { foreground = \"#a10004\" }\n\
error = { foreground = \"#a10005\" }\n\
warning = { foreground = \"#a10006\" }\n\
[components.picker]\n\
query = { foreground = \"#a20001\" }\n\
match = { foreground = \"#a20002\" }\n\
row = { foreground = \"#a20003\" }\n\
row_selected = { foreground = \"#a20004\", background = \"#020401\" }\n\
marker = { foreground = \"#a20009\" }\n\
badge_success = \"#a20005\"\n\
badge_warning = \"#a20006\"\n\
badge_error = \"#a20007\"\n\
border = \"#a20008\"\n\
[components.command_palette]\n\
query = { foreground = \"#a30002\" }\n\
row_selected = { foreground = \"#a30001\", background = \"#030401\" }\n\
[components.settings]\n\
row_selected = { foreground = \"#a40001\", background = \"#040401\" }\n\
marker = { foreground = \"#a40002\" }\n\
[components.group_form]\n\
label = { foreground = \"#ab0001\" }\n\
label_focused = { foreground = \"#ab0002\" }\n\
value = { foreground = \"#ab0003\" }\n\
value_focused = { foreground = \"#ab0004\" }\n\
marker = { foreground = \"#ab0005\" }\n\
[components.form]\n\
label = { foreground = \"#a50001\" }\n\
label_focused = { foreground = \"#a50002\" }\n\
label_editing = { foreground = \"#a50003\" }\n\
value = { foreground = \"#a50004\" }\n\
input = { foreground = \"#a50005\" }\n\
input_focused = { foreground = \"#a50006\" }\n\
input_editing = { foreground = \"#a50007\" }\n\
help = { foreground = \"#a50008\" }\n\
error = { foreground = \"#a50009\" }\n\
[components.table]\n\
row = { foreground = \"#a60001\" }\n\
row_selected = { foreground = \"#a60002\", background = \"#060401\" }\n\
[components.help]\n\
section = { foreground = \"#a70001\" }\n\
key = { foreground = \"#a70002\" }\n\
description = { foreground = \"#a70003\" }\n\
[components.keybind]\n\
row = { foreground = \"#a80001\" }\n\
row_selected = { foreground = \"#a80002\", background = \"#080401\" }\n\
marker = { foreground = \"#a80006\" }\n\
value = { foreground = \"#a80003\" }\n\
value_bound = { foreground = \"#a80004\" }\n\
value_capturing = { foreground = \"#a80005\" }\n\
[components.focus]\n\
indicator = { foreground = \"#a90001\" }\n\
[components.identities]\n\
empty = { foreground = \"#b00001\" }\n\
notice = { foreground = \"#b00002\" }\n\
[components.identities.card]\n\
border = \"#b10001\"\n\
border_selected = \"#b10002\"\n\
selection = { foreground = \"#b10003\", background = \"#0b0401\" }\n\
name = { foreground = \"#b10004\" }\n\
text = { foreground = \"#b10005\" }\n\
metadata = { foreground = \"#b10006\" }\n\
key_type = { foreground = \"#b10007\" }\n\
loaded = \"#b10008\"\n\
missing = \"#b10009\"\n\
credential = \"#b1000a\"\n\
[components.identities.agent]\n\
separator = \"#b20001\"\n\
label = { foreground = \"#b20002\" }\n\
value = { foreground = \"#b20003\" }\n\
count = { foreground = \"#b20004\" }\n\
[components.text]\n\
primary = { foreground = \"#c00001\" }\n\
bright = { foreground = \"#c00002\" }\n\
[components.status]\n\
success = \"#c10001\"\n\
[components.separator]\n\
primary = \"#c20001\"\n";

    /// A colour from [`OVERLAY_MARKERS`], by its hex digits.
    pub(crate) fn marker(hex: u32) -> ratatui::style::Color {
        ratatui::style::Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
    }

    /// An app wearing [`OVERLAY_MARKERS`] in `mode`.
    fn marked(mode: AppMode) -> App {
        let mut app = test_app_with_hosts();
        app.mode = mode;
        wear(&mut app, OVERLAY_MARKERS);
        app
    }

    /// The rect of the popup drawn this frame.
    fn drawn_popup(app: &App) -> Rect {
        app.last_popup_rect.get().expect("an overlay drew")
    }

    /// `default` plus `body`, resolved in memory. No filesystem, no HOME.
    pub(crate) fn overlay_theme(body: &str) -> crate::theme::model::ResolvedTheme {
        crate::test_support::resolved_source(
            "overlay-markers",
            &format!("schema_version = 1\nname = \"Markers\"\nextends = \"default\"\n{body}"),
        )
    }

    /// Give `app` a marker theme written as `[components.*]` TOML.
    pub(crate) fn wear(app: &mut App, body: &str) {
        app.activate_resolved_theme(std::rc::Rc::new(overlay_theme(body)));
    }

    /// The zoom toast is the one notice surface that inverts itself instead of
    /// writing into the status bar, so it has its own role — proved here with a
    /// marker no other role carries, through the real full-frame renderer, and
    /// pinned against the `theme::cyan() + REVERSED` cell it replaced.
    #[test]
    fn the_zoom_toast_wears_its_own_role() {
        const TOAST: u32 = 0xa5_0001;

        let mut app = test_app_with_hosts();
        app.panel_zoomed = true;
        app.host_notice = Some("copied 12 chars".into());
        wear(
            &mut app,
            &format!("[components.status_bar]\ntoast = {{ foreground = \"#{TOAST:06x}\" }}\n"),
        );
        let buf = render_to_buffer(&app, 80, 24);
        assert_eq!(
            style_at_text(&buf, "copied 12 chars").fg,
            Some(marker(TOAST)),
            "the zoom toast reads `components.status_bar.toast`"
        );

        // Under `default` the chip is still cyan, still inverted.
        let mut app = test_app_with_hosts();
        app.panel_zoomed = true;
        app.host_notice = Some("copied 12 chars".into());
        app.activate_resolved_theme(std::rc::Rc::new(crate::test_support::resolved_default()));
        let buf = render_to_buffer(&app, 80, 24);
        let style = style_at_text(&buf, "copied 12 chars");
        assert_eq!(
            style.fg,
            Some(crate::tui::theme::legacy::CYAN),
            "theme::cyan()"
        );
        assert!(
            style.add_modifier.contains(Modifier::REVERSED),
            "the chip is drawn inverted, as it always was"
        );
    }

    /// The style of the cell where `needle` starts.
    pub(crate) fn style_at_text(buf: &Buffer, needle: &str) -> ratatui::style::Style {
        let (x, y) = crate::test_support::find_text(buf, needle);
        cell_style(buf, x, y)
    }

    /// The style of the cell where `needle` starts *inside `rect`*.
    ///
    /// Overlays float over a dashboard that draws the same host names, so a
    /// whole-buffer search would happily read the wrong surface.
    pub(crate) fn style_at_text_in(
        buf: &Buffer,
        rect: Rect,
        needle: &str,
    ) -> ratatui::style::Style {
        for y in rect.y..rect.bottom() {
            let line: String = (rect.x..rect.right())
                .map(|x| buf[(x, y)].symbol())
                .collect();
            if let Some(byte) = line.find(needle) {
                return cell_style(buf, rect.x + line[..byte].chars().count() as u16, y);
            }
        }
        panic!("`{needle}` is not inside {rect:?}");
    }

    fn cell_style(buf: &Buffer, x: u16, y: u16) -> ratatui::style::Style {
        let cell = &buf[(x, y)];
        ratatui::style::Style::default()
            .fg(cell.fg)
            .bg(cell.bg)
            .add_modifier(cell.modifier)
    }

    /// The identities tab with one selected identity and `body`'s markers.
    fn identities_app(body: &str) -> App {
        let mut app = test_app_with_hosts();
        app.identities = vec![crate::store::Identity {
            id: 1,
            name: "prod-key".into(),
            username: Some("root".into()),
            private_key: Some(std::path::PathBuf::from("/home/u/.ssh/id_ed25519")),
            certificate: None,
            has_password: false,
        }];
        app.identity_selected = 0;
        app.active_tab = 3;
        wear(&mut app, body);
        app
    }

    /// The identity cards own `components.identities.*`. A migration that
    /// reached for the generic table roles instead would still look right under
    /// `default`, so both families are marked and only one may land.
    #[test]
    fn selected_identity_card_uses_identity_selection_not_table_highlight() {
        let app = identities_app(
            "[components.identities.card]\n\
             selection = { foreground = \"#ff0001\", background = \"#000101\" }\n\
             name = { foreground = \"#ff0002\" }\n\
             [components.table]\n\
             row_selected = { foreground = \"#00ff01\", background = \"#001100\" }\n",
        );
        let buf = render_to_buffer(&app, 120, 38);

        let name = style_at_text(&buf, "prod-key");
        assert_eq!(
            name.fg,
            Some(ratatui::style::Color::Rgb(0xff, 0x00, 0x02)),
            "the card name wears components.identities.card.name"
        );
        assert_eq!(
            name.bg,
            Some(ratatui::style::Color::Rgb(0x00, 0x01, 0x01)),
            "a selected card is backed by components.identities.card.selection"
        );
        assert_ne!(
            name.bg,
            Some(ratatui::style::Color::Rgb(0x00, 0x11, 0x00)),
            "the identity cards must not borrow components.table.row_selected"
        );
    }

    /// The confirm popups used to hard-code ANSI red and yellow. They now carry
    /// the popup's own semantic roles, and the two are marked apart so a single
    /// shared role cannot satisfy both.
    #[test]
    fn confirm_error_and_warning_use_popup_semantic_roles() {
        const MARKERS: &str = "[components.popup]\n\
             error = { foreground = \"#ff0003\" }\n\
             warning = { foreground = \"#ff0004\" }\n";
        let error_fg = Some(ratatui::style::Color::Rgb(0xff, 0x00, 0x03));
        let warning_fg = Some(ratatui::style::Color::Rgb(0xff, 0x00, 0x04));

        let mut app = test_app_with_hosts();
        app.pending_delete = Some(crate::app::PendingDelete::Host {
            id: 1,
            name: "web-prod".into(),
        });
        app.mode = AppMode::ConfirmDelete;
        wear(&mut app, MARKERS);
        let deleting = render_to_buffer(&app, 120, 38);
        let popup = app.last_popup_rect.get().expect("the delete popup drew");
        assert_eq!(
            Some(deleting[(popup.x, popup.y)].fg),
            error_fg,
            "a destructive confirm is framed by components.popup.error"
        );
        assert_eq!(
            style_at_text(&deleting, "Delete host").fg,
            warning_fg,
            "its question is components.popup.warning"
        );

        let mut app = test_app_with_hosts();
        app.mode = AppMode::ConfirmQuit;
        wear(&mut app, MARKERS);
        let quitting = render_to_buffer(&app, 120, 38);
        let popup = app.last_popup_rect.get().expect("the quit popup drew");
        assert_eq!(
            Some(quitting[(popup.x, popup.y)].fg),
            warning_fg,
            "a reversible confirm is framed by components.popup.warning"
        );
        assert_eq!(
            style_at_text(&quitting, "Quit sshub").fg,
            warning_fg,
            "its question is components.popup.warning too"
        );
    }

    // ── Release measurement: gradient rendering cost ────────────────
    //
    // Not a CI gate and not an isolation experiment. It asserts nothing about
    // timing — a shared runner's scheduling noise dwarfs the effect being
    // measured — and is `#[ignore]`d so only a deliberate local run produces
    // it.
    //
    // **What bounds the gradient pass is the gradient theme's own total frame
    // time**, not the printed delta. The pass runs serially inside the frame,
    // so it cannot cost more than the whole frame takes; a `fire` frame around
    // a quarter of a millisecond is therefore already a conservative bound far
    // under the spec's `2 ms` criterion, whatever share of it the gradients
    // actually are.
    //
    // The **delta is not a bound on anything**. It compares two differently
    // configured themes, always in the same order: `fire` differs from
    // `high-contrast` in every value it sets, faster work elsewhere in `fire`
    // can offset gradient work inside the difference, and `saturating_sub`
    // clamps a negative observation to zero — which has been observed, with the
    // gradient side measuring the faster of the two. Treat it as a
    // non-isolated smoke observation and nothing more.
    //
    // Isolating the pass properly would mean measuring one app state twice,
    // once with gradient paints and once with equivalent solid ones, with the
    // order alternated between runs. The numbers are recorded in
    // `docs/theme-render-benchmark.md`.

    /// Frames measured per theme. The spec asks for at least 1,000.
    const BENCH_SAMPLES: usize = 1_000;
    /// Frames rendered before the clock starts, per theme.
    const BENCH_WARMUP: usize = 100;

    /// Render `samples` frames of `app` at `200x60` and return the durations.
    fn bench_frames(app: &App, warmup: usize, samples: usize) -> Vec<std::time::Duration> {
        let backend = TestBackend::new(200, 60);
        let mut terminal = Terminal::new(backend).unwrap();
        for _ in 0..warmup {
            terminal.draw(|frame| render(frame, app)).unwrap();
        }
        let mut durations = Vec::with_capacity(samples);
        for _ in 0..samples {
            let started = std::time::Instant::now();
            terminal.draw(|frame| render(frame, app)).unwrap();
            durations.push(started.elapsed());
        }
        durations
    }

    fn median(durations: &mut [std::time::Duration]) -> std::time::Duration {
        durations.sort_unstable();
        durations[durations.len() / 2]
    }

    /// Print the median frame time of a solid theme, of a gradient theme, and
    /// the difference. Never asserts on time.
    ///
    /// `high-contrast` is the solid side because it is the closest comparison
    /// the built-ins offer: it paints an opaque app background like `fire` does
    /// but defines no gradient at all, so at least the presence of a background
    /// pass is not what separates them. They still differ in every other value
    /// they set, and they are always measured in that order, so the printed
    /// delta is a smoke observation, not a bound. The bound is the gradient
    /// theme's own frame time, which is printed alongside it.
    #[test]
    #[ignore = "local release measurement; prints timings, asserts none"]
    fn theme_gradient_release_benchmark() {
        let solid = app_with_builtin_theme("high-contrast");
        let gradient = app_with_builtin_theme("fire");
        assert!(
            solid.theme().gradients().is_empty(),
            "the solid side must define no gradients"
        );
        assert!(
            !gradient.theme().gradients().is_empty(),
            "the gradient side must define gradients"
        );

        let mut solid_times = bench_frames(&solid, BENCH_WARMUP, BENCH_SAMPLES);
        let mut gradient_times = bench_frames(&gradient, BENCH_WARMUP, BENCH_SAMPLES);
        let solid_median = median(&mut solid_times);
        let gradient_median = median(&mut gradient_times);
        let delta = gradient_median.saturating_sub(solid_median);

        println!("theme gradient render benchmark  (200x60, {BENCH_SAMPLES} frames each, {BENCH_WARMUP} warm-up)");
        println!(
            "  solid    (high-contrast) median: {:.3} ms",
            solid_median.as_secs_f64() * 1e3
        );
        println!(
            "  gradient (fire)          median: {:.3} ms",
            gradient_median.as_secs_f64() * 1e3
        );
        println!(
            "  delta (NOT a bound; see below) : {:.3} ms",
            delta.as_secs_f64() * 1e3
        );
        // The claim that survives review: the gradient pass runs serially
        // inside the gradient frame, so the whole frame bounds it from above.
        println!(
            "  => the gradient pass costs at most one whole `fire` frame, {:.3} ms",
            gradient_median.as_secs_f64() * 1e3
        );
        println!(
            "     (the delta compares two different themes in fixed order and bounds nothing)"
        );
    }
}
