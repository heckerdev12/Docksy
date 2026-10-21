#![windows_subsystem = "windows"]

use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const BAR_HEIGHT_DIP: i32 = 32;
const APPBAR_CALLBACK: u32 = WM_USER + 1;
const TIMER_ID: usize = 1;

// Kept so the panic hook can release the reserved screen space.
static BAR_HWND: AtomicIsize = AtomicIsize::new(0);

fn main() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // If we crash, still give the screen space back.
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let raw = BAR_HWND.load(Ordering::SeqCst);
            if raw != 0 {
                unregister_appbar(HWND(raw as *mut _));
            }
            default_hook(info);
        }));

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = w!("LintelBar");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST, // no taskbar button, stays on top
            class,
            w!("Lintel"),
            WS_POPUP,
            0, 0, 0, 0,
            None, None, hinst, None,
        )?;
        BAR_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

        register_appbar(hwnd);
        SetTimer(hwnd, TIMER_ID, 1000, None);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

fn bar_height_px() -> i32 {
    unsafe { BAR_HEIGHT_DIP * GetDpiForSystem() as i32 / 96 }
}

unsafe fn appbar_data(hwnd: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        ..Default::default()
    }
}

unsafe fn register_appbar(hwnd: HWND) {
    let mut abd = appbar_data(hwnd);
    abd.uCallbackMessage = APPBAR_CALLBACK;
    SHAppBarMessage(ABM_NEW, &mut abd);
    position_appbar(hwnd);
}

// Ask Windows for the top edge, then claim exactly bar-height pixels of it.
unsafe fn position_appbar(hwnd: HWND) {
    let mut abd = appbar_data(hwnd);
    abd.uEdge = ABE_TOP;
    abd.rc = RECT {
        left: 0,
        top: 0,
        right: GetSystemMetrics(SM_CXSCREEN),
        bottom: GetSystemMetrics(SM_CYSCREEN),
    };
    SHAppBarMessage(ABM_QUERYPOS, &mut abd);
    abd.rc.bottom = abd.rc.top + bar_height_px();
    SHAppBarMessage(ABM_SETPOS, &mut abd);
    let _ = MoveWindow(
        hwnd,
        abd.rc.left,
        abd.rc.top,
        abd.rc.right - abd.rc.left,
        abd.rc.bottom - abd.rc.top,
        true,
    );
}

unsafe fn unregister_appbar(hwnd: HWND) {
    let mut abd = appbar_data(hwnd);
    SHAppBarMessage(ABM_REMOVE, &mut abd);
}

unsafe fn active_window_title(own: HWND) -> String {
    let fg = GetForegroundWindow();
    if fg.0.is_null() || fg == own {
        return String::new();
    }
    let mut buf = [0u16; 256];
    let n = GetWindowTextW(fg, &mut buf);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

unsafe fn draw_text(hdc: HDC, text: &str, rc: &mut RECT, flags: DRAW_TEXT_FORMAT) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    if !wide.is_empty() {
        DrawTextW(hdc, &mut wide, rc, flags);
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);

    // COLORREF is 0x00BBGGRR. #0e1013 -> 0x0013100e
    let bg = CreateSolidBrush(COLORREF(0x0013_100e));
    FillRect(hdc, &rc, bg);
    let _ = DeleteObject(bg);

    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, COLORREF(0x00f3_f0ee));
    SelectObject(hdc, GetStockObject(DEFAULT_GUI_FONT));

    // Left: active window title
    let mut left = RECT { left: rc.left + 16, right: rc.right / 3, ..rc };
    draw_text(
        hdc,
        &active_window_title(hwnd),
        &mut left,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    // Center: clock
    let t = GetLocalTime();
    let clock = format!("{:02}/{:02}   {:02}:{:02}", t.wDay, t.wMonth, t.wHour, t.wMinute);
    let mut center = rc;
    draw_text(hdc, &clock, &mut center, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    let _ = EndPaint(hwnd, &ps);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = InvalidateRect(hwnd, None, false);
                LRESULT(0)
            }
            APPBAR_CALLBACK => {
                if wp.0 as u32 == ABN_POSCHANGED {
                    position_appbar(hwnd);
                }
                LRESULT(0)
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                position_appbar(hwnd);
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                // v0.1 quit: right-click the bar.
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                unregister_appbar(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
