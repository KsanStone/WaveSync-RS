//! Windows-only custom caption. DWM still supplies the border, shadow and corners.
use egui::emath::GuiRounding;
use egui::{Color32, Rect, Stroke};
use std::{cell::Cell, sync::Arc};
use windows::Win32::{
    Foundation::{BOOL, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
        Dwm::{
            DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
            DwmSetWindowAttribute,
        },
        Gdi::{InvalidateRect, ScreenToClient},
    },
    UI::{
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

const SUBCLASS_ID: usize = 0x5753594e;
const HEIGHT: f32 = 32.0;
const BUTTON_WIDTH: f32 = 46.0;

struct Frame {
    hwnd: HWND,
    fullscreen: Cell<bool>,
    scale: Cell<f32>,
    hover: Cell<u32>,
}
impl Frame {
    fn redraw(&self) {
        // Do not call winit while in its WndProc: it may hold its window-state lock.
        unsafe {
            let _ = InvalidateRect(self.hwnd, None, false);
        }
    }
}
fn hwnd(window: &Window) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as _)),
        _ => None,
    }
}
pub(crate) fn install(window: &Arc<Window>) {
    let Some(handle) = hwnd(window) else { return };
    let frame = Box::new(Frame {
        hwnd: handle,
        fullscreen: Cell::new(window.fullscreen().is_some()),
        scale: Cell::new(window.scale_factor() as f32),
        hover: Cell::new(0),
    });
    unsafe {
        let data = Box::into_raw(frame);
        if !SetWindowSubclass(handle, Some(frame_proc), SUBCLASS_ID, data as usize).as_bool() {
            drop(Box::from_raw(data));
            log::error!("Could not install custom titlebar");
            return;
        }
        let _ = SetPropW(
            handle,
            windows::core::w!("WaveSync.CustomFrame"),
            HANDLE(data.cast()),
        );
        // Keep WS_CAPTION/WS_THICKFRAME and the native system-button styles.
        // WM_NCCALCSIZE removes the visible native caption, while these styles
        // let Windows retain its normal caption commands and DWM transitions.
        let dark = BOOL(1);
        let _ = DwmSetWindowAttribute(
            handle,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const BOOL).cast(),
            size_of::<BOOL>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            handle,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&DWMWCP_ROUND as *const windows::Win32::Graphics::Dwm::DWM_WINDOW_CORNER_PREFERENCE)
                .cast(),
            size_of_val(&DWMWCP_ROUND) as u32,
        );
        let _ = SetWindowPos(
            handle,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        );
    }
}
fn is_button(hit: u32) -> bool {
    matches!(hit, HTMINBUTTON | HTMAXBUTTON | HTCLOSE)
}
fn caption_hit(width: f32, x: f32, y: f32) -> u32 {
    if y < 0.0 || y >= HEIGHT || x < 0.0 || x >= width {
        return HTCLIENT;
    }
    if x >= width - BUTTON_WIDTH {
        HTCLOSE
    } else if x >= width - BUTTON_WIDTH * 2.0 {
        HTMAXBUTTON
    } else if x >= width - BUTTON_WIDTH * 3.0 {
        HTMINBUTTON
    } else {
        HTCAPTION
    }
}

pub(crate) fn titlebar(ctx: &egui::Context, window: &Window) {
    let Some(handle) = hwnd(window) else { return };
    let data = unsafe { GetPropW(handle, windows::core::w!("WaveSync.CustomFrame")) };
    if data.is_invalid() {
        return;
    }
    let frame = unsafe { &*(data.0 as *const Frame) };
    frame.fullscreen.set(window.fullscreen().is_some());
    if frame.fullscreen.get() {
        return;
    }
    frame.scale.set(ctx.pixels_per_point());
    // Reserve layout space without an egui input region over the non-client caption.
    let rect = egui::TopBottomPanel::top("custom_windows_titlebar")
        .exact_height(HEIGHT)
        .frame(egui::Frame::NONE)
        .show(ctx, |_| {})
        .response
        .rect;
    // The app paints its full-window backdrop later on the background layer.
    let painter = egui::Painter::new(
        ctx.clone(),
        egui::LayerId::new(egui::Order::Foreground, egui::Id::new("window_caption")),
        rect,
    );
    let title_rect = Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right() - BUTTON_WIDTH * 3.0, rect.bottom()),
    );
    let title_painter = painter.with_clip_rect(title_rect);
    let mut title_x = rect.left() + 12.0;
    if let Some(icon) =
        ctx.data(|data| data.get_temp::<egui::TextureHandle>(egui::Id::new("window_caption_icon")))
    {
        title_painter.image(
            icon.id(),
            Rect::from_center_size(
                egui::pos2(title_x + 8.0, rect.center().y),
                egui::vec2(16.0, 16.0),
            ),
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        title_x += 24.0;
    }
    title_painter.text(
        egui::pos2(title_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        window.title(),
        egui::FontId::proportional(12.0),
        Color32::from_gray(235),
    );
    for (index, hit) in [HTMINBUTTON, HTMAXBUTTON, HTCLOSE].into_iter().enumerate() {
        let button = Rect::from_min_size(
            egui::pos2(rect.right() - BUTTON_WIDTH * (3 - index) as f32, rect.top()),
            egui::vec2(BUTTON_WIDTH, HEIGHT),
        );
        let hover = frame.hover.get() == hit;
        if hover {
            let fill = if hit == HTCLOSE {
                Color32::from_rgb(196, 43, 28)
            } else {
                Color32::from_white_alpha(45)
            };
            painter.rect_filled(button, 0.0, fill);
        }
        let center = button.center().round_to_pixels(ctx.pixels_per_point());
        let stroke = Stroke::new(1.0 / ctx.pixels_per_point(), Color32::from_gray(240));
        let p = |x, y| center + egui::vec2(x, y);
        match hit {
            HTMINBUTTON => {
                painter.line_segment([p(-5.0, 0.0), p(5.0, 0.0)], stroke);
            }
            HTCLOSE => {
                painter.line_segment([p(-5.0, -5.0), p(5.0, 5.0)], stroke);
                painter.line_segment([p(5.0, -5.0), p(-5.0, 5.0)], stroke);
            }
            _ => {
                if window.is_maximized() {
                    painter.line_segment([p(-3.0, -5.0), p(5.0, -5.0)], stroke);
                    painter.line_segment([p(5.0, -5.0), p(5.0, 3.0)], stroke);
                    painter.line_segment([p(-3.0, -5.0), p(-3.0, -3.0)], stroke);
                    painter.line_segment([p(3.0, 3.0), p(5.0, 3.0)], stroke);
                    painter.rect_stroke(
                        Rect::from_min_max(p(-5.0, -3.0), p(3.0, 5.0)),
                        0.0,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                } else {
                    painter.rect_stroke(
                        Rect::from_center_size(center, egui::vec2(10.0, 10.0)),
                        0.0,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
    }
}
unsafe extern "system" fn frame_proc(
    window: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(window, Some(frame_proc), SUBCLASS_ID);
            let result = DefSubclassProc(window, msg, wparam, lparam);
            let _ = RemovePropW(window, windows::core::w!("WaveSync.CustomFrame"));
            drop(Box::from_raw(data as *mut Frame));
            return result;
        }
        let frame = &*(data as *const Frame);
        if frame.fullscreen.get() {
            return DefSubclassProc(window, msg, wparam, lparam);
        }
        match msg {
            // Maximization/activation can request a fresh native caption paint
            // independently of button mouse tracking. We own caption pixels;
            // keep native commands and DWM's border/shadow, but not that painting.
            WM_NCPAINT => return LRESULT(0),
            WM_NCACTIVATE => return DefSubclassProc(window, msg, wparam, LPARAM(-1)),
            WM_NCCALCSIZE => {
                // Keep only a one-pixel non-client outline when restored. The
                // full native sizing border would otherwise remain outside our
                // GL surface and show as unpainted strips with custom NC paint.
                if wparam.0 == 0 {
                    return LRESULT(0);
                }
                let params = lparam.0 as *mut NCCALCSIZE_PARAMS;
                let outer = (*params).rgrc[0];
                let result = DefSubclassProc(window, msg, wparam, lparam);
                if result.0 != 0 {
                    return result;
                }
                (*params).rgrc[0].top = outer.top;
                if IsZoomed(window).as_bool() {
                    let dpi = GetDpiForWindow(window);
                    (*params).rgrc[0].top += GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi)
                        + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                } else {
                    (*params).rgrc[0].left = outer.left + 1;
                    (*params).rgrc[0].right = outer.right - 1;
                    (*params).rgrc[0].bottom = outer.bottom - 1;
                }
                return LRESULT(0);
            }
            WM_NCHITTEST => {
                let mut point = POINT {
                    x: lparam.0 as u16 as i16 as i32,
                    y: (lparam.0 >> 16) as u16 as i16 as i32,
                };
                let _ = ScreenToClient(window, &mut point);
                let mut rect = RECT::default();
                let _ = GetClientRect(window, &mut rect);
                let scale = frame.scale.get();
                if !IsZoomed(window).as_bool() {
                    let dpi = GetDpiForWindow(window);
                    let border = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi)
                        + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                    let left = point.x < border;
                    let right = point.x >= rect.right - border;
                    let top = point.y < border;
                    let bottom = point.y >= rect.bottom - border;
                    let hit = match (left, right, top, bottom) {
                        (true, _, true, _) => HTTOPLEFT,
                        (_, true, true, _) => HTTOPRIGHT,
                        (true, _, _, true) => HTBOTTOMLEFT,
                        (_, true, _, true) => HTBOTTOMRIGHT,
                        (true, _, _, _) => HTLEFT,
                        (_, true, _, _) => HTRIGHT,
                        (_, _, true, _) => HTTOP,
                        (_, _, _, true) => HTBOTTOM,
                        _ => HTCLIENT,
                    };
                    if hit != HTCLIENT {
                        return LRESULT(hit as isize);
                    }
                }
                return LRESULT(caption_hit(
                    rect.right as f32 / scale,
                    point.x as f32 / scale,
                    point.y as f32 / scale,
                ) as isize);
            }
            WM_NCMOUSEMOVE => {
                let hit = wparam.0 as u32;
                if frame.hover.replace(hit) != hit {
                    frame.redraw();
                }
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: window,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut track);
                // Consume button hover, as in the Java app: default caption
                // tracking would paint native controls on top of our controls.
                if is_button(hit) {
                    return LRESULT(0);
                }
            }
            WM_NCMOUSELEAVE | WM_MOUSEMOVE => {
                if frame.hover.replace(0) != 0 {
                    frame.redraw();
                }
            }
            WM_NCLBUTTONDOWN if is_button(wparam.0 as u32) => {
                // Use the native animated command, not native button tracking.
                // This is the same dispatch used by Java's CustomizedStage.
                let command = match wparam.0 as u32 {
                    HTMINBUTTON => SC_MINIMIZE,
                    HTMAXBUTTON if IsZoomed(window).as_bool() => SC_RESTORE,
                    HTMAXBUTTON => SC_MAXIMIZE,
                    _ => SC_CLOSE,
                };
                return SendMessageW(window, WM_SYSCOMMAND, WPARAM(command as usize), LPARAM(0));
            }
            WM_NCLBUTTONUP | WM_NCLBUTTONDBLCLK if is_button(wparam.0 as u32) => {
                return LRESULT(0);
            }
            _ => {}
        }
        DefSubclassProc(window, msg, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caption_geometry_matches_fixed_height_and_button_order() {
        assert_eq!(caption_hit(1000.0, 100.0, 31.0), HTCAPTION);
        assert_eq!(caption_hit(1000.0, 100.0, 32.0), HTCLIENT);
        assert_eq!(caption_hit(1000.0, 870.0, 16.0), HTMINBUTTON);
        assert_eq!(caption_hit(1000.0, 930.0, 16.0), HTMAXBUTTON);
        assert_eq!(caption_hit(1000.0, 980.0, 16.0), HTCLOSE);
        assert_eq!(caption_hit(1000.0, 1000.0, 16.0), HTCLIENT);
    }
}
