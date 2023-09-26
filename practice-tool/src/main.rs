// johndisandonato's Elden Ring Practice Tool
// Copyright (C) 2022  johndisandonato <https://github.com/veeenu>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::mem;
use std::os::windows::prelude::{AsRawHandle, OsStringExt};
use std::path::PathBuf;

use dll_syringe::process::OwnedProcess;
use dll_syringe::Syringe;
use hudhook::tracing::{debug, trace};
use pkg_version::*;
use semver::*;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;
use windows::core::{w, PCSTR, PWSTR};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::System::Threading::{QueryFullProcessImageNameW, PROCESS_NAME_FORMAT};
use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OPENFILENAMEW, OPEN_FILENAME_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxA, MessageBoxW, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_YESNO,
};

fn err_to_string<T: std::fmt::Display>(e: T) -> String {
    format!("Error: {}", e)
}

fn get_current_version() -> Version {
    Version {
        major: pkg_version_major!(),
        minor: pkg_version_minor!(),
        patch: pkg_version_patch!(),
        pre: Prerelease::EMPTY,
        build: BuildMetadata::EMPTY,
    }
}

fn get_latest_version() -> Result<(Version, String, String), String> {
    #[derive(serde::Deserialize)]
    struct GithubRelease {
        tag_name: String,
        html_url: String,
        body: String,
    }

    let release =
        ureq::get("https://api.github.com/repos/veeenu/eldenring-practice-tool/releases/latest")
            .call()
            .map_err(|e| format!("{}", e))?
            .into_json::<GithubRelease>()
            .map_err(|e| format!("{}", e))?;

    let version = Version::parse(&release.tag_name).map_err(err_to_string)?;

    Ok((version, release.html_url, release.body))
}

fn check_eac(handle: HANDLE) -> Result<bool, String> {
    let mut buf = [0u16; 256];
    let mut len = 256u32;
    let exe_path = PWSTR(buf.as_mut_ptr());
    unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_FORMAT(0), exe_path, &mut len) }
        .map_err(|e| format!("{e}"))?;
    let exe_path = PathBuf::from(unsafe { exe_path.to_string() }.map_err(|e| format!("{e}"))?);
    let exe_cwd = exe_path.parent().unwrap(); // Unwrap ok: must be in a Game directory anyway

    let steam_appid_path = exe_cwd.join("steam_appid.txt");
    debug!("{steam_appid_path:?} {}", steam_appid_path.exists());
    if !steam_appid_path.exists() {
        unsafe {
            let text = w!("The practice tool can't start if the EAC bypass is not applied.\n\nNo \
                           worries! I can apply that for you.\n\nPlease close the game, press \
                           \"Ok\", and pick the eldenring.exe file you want to apply the bypass \
                           to.");
            let caption = w!("EAC was not bypassed");
            MessageBoxW(HWND(0), text, caption, MB_ICONERROR);

            let mut file_path = [0u16; 256];
            let mut open_file_name = OPENFILENAMEW {
                lStructSize: mem::size_of::<OPENFILENAMEW>() as u32,
                lpstrFilter: w!("Elden Ring executable (eldenring.exe)\0eldenring.exe\0\0"),
                nMaxCustFilter: 0,
                nFilterIndex: 0,
                lpstrFile: PWSTR(file_path.as_mut_ptr()),
                nMaxFile: 256,
                nMaxFileTitle: 0,
                Flags: OPEN_FILENAME_FLAGS(0),
                nFileOffset: 0,
                nFileExtension: 0,
                ..Default::default()
            };

            if GetOpenFileNameW(&mut open_file_name).as_bool() {
                let exe_path = PathBuf::from(OsString::from_wide(&file_path));
                // Unwrap ok: must be in a Game directory anyway
                let steam_appid_path = exe_path.parent().unwrap().join("steam_appid.txt");
                let mut file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(steam_appid_path)
                    .map_err(|e| format!("Couldn't open steam_appid.txt: {e}"))?;
                file.write_all(b"1245620")
                    .map_err(|e| format!("Couldn't write steam_appid.txt: {e}"))?;

                let text = w!("EAC is now bypassed. You can now restart the game and the tool.");
                let caption = w!("EAC bypassed");
                MessageBoxW(HWND(0), text, caption, MB_ICONINFORMATION);
            } else {
                let text = w!("EAC bypass was not applied. Please either re-run the tool to \
                               automatically apply the bypass, or apply it manually.\n\
                               Read more at:\nhttps://wiki.speedsouls.com/eldenring:Downpatching.");
                let caption = w!("EAC was not bypassed");
                MessageBoxW(HWND(0), text, caption, MB_ICONERROR);
            }

            return Ok(true);
        }
    }

    Ok(false)
}

fn perform_injection() -> Result<(), String> {
    let mut dll_path = std::env::current_exe().unwrap();
    dll_path.pop();
    dll_path.push("jdsd_er_practice_tool.dll");

    if !dll_path.exists() {
        dll_path.pop();
        dll_path.push("libjdsd_er_practice_tool");
        dll_path.set_extension("dll");
    }

    let dll_path = dll_path.canonicalize().map_err(err_to_string)?;
    trace!("Injecting {:?}", dll_path);

    let process = OwnedProcess::find_first_by_name("eldenring.exe")
        .ok_or_else(|| "Could not find process".to_string())?;

    trace!("Checking EAC...");
    if check_eac(HANDLE(process.as_raw_handle() as _))? {
        return Ok(());
    }

    let syringe = Syringe::for_process(process);
    syringe.inject(dll_path).map_err(|e| {
        format!(
            "Could not hook the practice tool: {e}.\n\nPlease make sure you have no antiviruses \
             running, EAC is properly bypassed, and you are running an unmodded and legitimate \
             version of the game."
        )
    })?;

    Ok(())
}

fn main() {
    {
        let stdout_layer = tracing_subscriber::fmt::layer()
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .with_thread_names(true)
            .with_ansi(true)
            .boxed();

        tracing_subscriber::registry().with(LevelFilter::TRACE).with(stdout_layer).init();
    }

    let current_version = get_current_version();

    match get_latest_version() {
        Ok((latest_version, download_url, release_notes)) => {
            if latest_version > current_version {
                let update_msg = format!(
                    "A new version of the practice tool is available!\n\nLatest version: \
                     {}\nInstalled version: {}\n\nRelease notes:\n{}\n\nDo you want to download \
                     the update?\0",
                    latest_version, current_version, release_notes
                );

                let msgbox_response = unsafe {
                    MessageBoxA(
                        HWND(0),
                        PCSTR(update_msg.as_str().as_ptr()),
                        PCSTR("Update available\0".as_ptr()),
                        MB_YESNO | MB_ICONINFORMATION,
                    )
                };

                if IDYES == msgbox_response {
                    open::that(download_url).ok();
                }
            }
        },
        Err(e) => {
            let error_msg = format!("Could not check for a new version: {}\0", e);
            unsafe {
                MessageBoxA(
                    HWND(0),
                    PCSTR(error_msg.as_str().as_ptr()),
                    PCSTR("Error\0".as_ptr()),
                    MB_OK | MB_ICONERROR,
                );
            }
        },
    }

    if let Err(e) = perform_injection() {
        let error_msg = format!("{}\0", e);
        debug!("{e}");
        unsafe {
            MessageBoxA(
                HWND(0),
                PCSTR(error_msg.as_str().as_ptr()),
                PCSTR("Error\0".as_ptr()),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}
