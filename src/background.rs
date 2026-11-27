use anyhow::{Context, Result};
use gpui::{App, Global};
#[cfg(not(target_os = "windows"))]
use std::{fs, path::PathBuf};
use tray_icon::{
    TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};

const OPEN: &str = "fero.open";
const QUIT: &str = "fero.quit";

pub struct Tray {
    _icon: TrayIcon,
}

impl Global for Tray {}

impl Tray {
    pub fn install(cx: &mut App) -> Result<()> {
        #[cfg(target_os = "macos")]
        let icon = include_bytes!("../design/brand/tray-template.png");
        #[cfg(not(target_os = "macos"))]
        let icon = include_bytes!("../design/brand/tray-color.png");
        let icon = image::load_from_memory_with_format(icon, image::ImageFormat::Png)?.into_rgba8();
        let (width, height) = icon.dimensions();
        let menu = Menu::new();
        menu.append_items(&[
            &MenuItem::with_id(OPEN, "Open Fero", true, None),
            &MenuItem::with_id(QUIT, "Quit Fero", true, None),
        ])?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(tray_icon::Icon::from_rgba(icon.into_raw(), width, height)?)
            .with_icon_as_template(cfg!(target_os = "macos"))
            .with_tooltip("Fero")
            .build()?;
        cx.set_global(Self { _icon: tray });
        cx.spawn(async |cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                while let Ok(event) = MenuEvent::receiver().try_recv() {
                    if cx
                        .update(|cx| match event.id.as_ref() {
                            OPEN => crate::show_window(cx),
                            QUIT => cx.quit(),
                            _ => {}
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
        })
        .detach();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn startup_file() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("Cannot find home folder")?
        .join("Library/LaunchAgents/dev.fero.desktop.plist"))
}

#[cfg(target_os = "linux")]
fn startup_file() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("Cannot find configuration folder")?
        .join("autostart/dev.fero.desktop.desktop"))
}

#[cfg(not(target_os = "windows"))]
pub fn starts_at_login() -> Result<bool> {
    Ok(startup_file()?.try_exists()?)
}

#[cfg(not(target_os = "windows"))]
pub fn set_start_at_login(enabled: bool) -> Result<()> {
    let path = startup_file()?;
    if enabled {
        // AppImage's current executable lives in a temporary mount which is
        // gone at the next login. The runtime supplies the original image path.
        #[cfg(target_os = "linux")]
        let image = std::env::var_os("APPIMAGE").map(PathBuf::from);
        #[cfg(target_os = "macos")]
        let image: Option<PathBuf> = None;
        let executable = match image {
            Some(path) => path,
            None => std::env::current_exe()?,
        };
        let executable = executable
            .to_str()
            .context("Fero's executable path must be valid UTF-8")?;
        fs::create_dir_all(path.parent().context("Missing startup folder")?)?;
        let temporary = path.with_extension("tmp");
        write_startup_file(&temporary, executable)?;
        fs::rename(temporary, path)?;
    } else if let Err(error) = fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(error.into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn write_startup_file(path: &std::path::Path, executable: &str) -> Result<()> {
    use plist::{Dictionary, Value};
    let mut agent = Dictionary::new();
    agent.insert("Label".into(), "dev.fero.desktop".into());
    agent.insert("RunAtLoad".into(), true.into());
    agent.insert("LimitLoadToSessionType".into(), "Aqua".into());
    agent.insert(
        "ProgramArguments".into(),
        Value::Array(vec![executable.into(), "--background".into()]),
    );
    Ok(Value::Dictionary(agent).to_file_xml(path)?)
}

#[cfg(target_os = "linux")]
fn write_startup_file(path: &std::path::Path, executable: &str) -> Result<()> {
    anyhow::ensure!(
        !executable.contains(['\n', '\r']),
        "Unsupported executable path"
    );
    // First escape quoted Exec arguments, then the desktop entry string itself.
    let mut escaped = String::new();
    for ch in executable.chars() {
        match ch {
            '\\' | '"' | '`' | '$' => escaped.push('\\'),
            '%' => escaped.push('%'),
            _ => {}
        }
        escaped.push(ch);
    }
    let escaped = escaped.replace('\\', "\\\\");
    Ok(fs::write(
        path,
        format!("[Desktop Entry]\nType=Application\nName=Fero\nExec=\"{escaped}\" --background\n"),
    )?)
}

#[cfg(target_os = "windows")]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(target_os = "windows")]
pub fn starts_at_login() -> Result<bool> {
    use std::{os::windows::process::CommandExt, process::Command};
    Ok(Command::new("reg.exe")
        .creation_flags(0x0800_0000)
        .args(["query", RUN_KEY, "/v", "Fero"])
        .output()?
        .status
        .success())
}

#[cfg(target_os = "windows")]
pub fn set_start_at_login(enabled: bool) -> Result<()> {
    use std::{os::windows::process::CommandExt, process::Command};
    let mut command = Command::new("reg.exe");
    command.creation_flags(0x0800_0000);
    if enabled {
        let executable = std::env::current_exe()?;
        let executable = executable
            .to_str()
            .context("Fero's executable path must be valid UTF-8")?;
        command.args([
            "add",
            RUN_KEY,
            "/v",
            "Fero",
            "/t",
            "REG_SZ",
            "/d",
            &format!("\"{executable}\" --background"),
            "/f",
        ]);
    } else if starts_at_login()? {
        command.args(["delete", RUN_KEY, "/v", "Fero", "/f"]);
    } else {
        return Ok(());
    }
    let output = command.output()?;
    anyhow::ensure!(
        output.status.success(),
        "Could not update Windows start at login"
    );
    Ok(())
}
