#![windows_subsystem = "windows"]

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use bin_reader::{normalize_path, BinaryReader};

pub mod bin_reader;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
const APP_METADATA: &[u8] = include_bytes!("../app_metadata.toml");
#[cfg(not(windows))]
const APP_METADATA: &[u8] = &[];

const APP_METADATA_CONFIG: &str = "meta.toml";
const META_LINE_PREFIX_TIMESTAMP: &str = "timestamp = ";
const META_LINE_PREFIX_FILE: &str = "file = ";
const APP_PREFIX: &str = "rustdesk";
const APPNAME_RUNTIME_ENV_KEY: &str = "RUSTDESK_APPNAME";

const EMMEEFFE_APPNAME: &str =
    "rustdesk-host=pbxemmeeffe.dyndns.org,key=4WTIAYQnc4BddzGns6tfwJimRXsKlsKmOjjWHegNtns=.exe";

#[cfg(windows)]
const SET_FOREGROUND_WINDOW_ENV_KEY: &str = "SET_FOREGROUND_WINDOW";

fn app_dir_name(exe: &str) -> String {
    Path::new(&exe.replace('\\', "/"))
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.trim().to_lowercase())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| APP_PREFIX.to_owned())
}

fn is_timestamp_matches(dir: &Path, ts: &mut u64) -> bool {
    let Ok(app_metadata) = std::str::from_utf8(APP_METADATA) else {
        return true;
    };

    for line in app_metadata.lines() {
        if line.starts_with(META_LINE_PREFIX_TIMESTAMP) {
            if let Ok(stored_ts) =
                line.replace(META_LINE_PREFIX_TIMESTAMP, "").parse::<u64>()
            {
                *ts = stored_ts;
                break;
            }
        }
    }

    if *ts == 0 {
        return true;
    }

    if let Ok(content) = std::fs::read_to_string(dir.join(APP_METADATA_CONFIG)) {
        for line in content.lines() {
            if line.starts_with(META_LINE_PREFIX_TIMESTAMP) {
                if let Ok(stored_ts) =
                    line.replace(META_LINE_PREFIX_TIMESTAMP, "").parse::<u64>()
                {
                    return *ts == stored_ts;
                }
            }
        }
    }

    false
}

fn write_meta(dir: &Path, ts: u64, package_paths: &[String]) {
    let meta_file = dir.join(APP_METADATA_CONFIG);
    let mut content = format!("{}{}\n", META_LINE_PREFIX_TIMESTAMP, ts);

    for path in package_paths {
        content.push_str(&format!("{}{}\n", META_LINE_PREFIX_FILE, path));
    }

    let _ = std::fs::write(meta_file, content);
}

fn previous_package_files(dir: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(dir.join(APP_METADATA_CONFIG)) else {
        return Vec::new();
    };

    content
        .lines()
        .filter_map(|line| line.strip_prefix(META_LINE_PREFIX_FILE))
        .map(|path| path.trim().to_owned())
        .collect()
}

fn resolve_within(dir: &Path, relative: &str) -> Option<PathBuf> {
    use std::path::Component;

    let mut path = dir.to_path_buf();
    let mut any = false;

    for component in Path::new(&relative.replace('\\', "/")).components() {
        match component {
            Component::Normal(part) => {
                if part.to_string_lossy().contains(':') {
                    return None;
                }

                path.push(part);
                any = true;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }

    if any {
        Some(path)
    } else {
        None
    }
}

fn remove_dropped_package_files_with<F>(
    dir: &Path,
    current: &[String],
    mut remove_file: F,
) -> Vec<String>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let keep: std::collections::HashSet<String> =
        current.iter().map(|p| normalize_path(p)).collect();

    let mut failed = Vec::new();

    for previous in previous_package_files(dir) {
        if keep.contains(&normalize_path(&previous)) {
            continue;
        }

        let Some(path) = resolve_within(dir, &previous) else {
            continue;
        };

        if path.is_file() {
            println!("removing dropped {}", previous);

            if let Err(error) = remove_file(&path) {
                eprintln!("failed to remove dropped {}: {}", previous, error);
                failed.push(previous);
            }
        }
    }

    failed
}

fn remove_dropped_package_files(dir: &Path, current: &[String]) -> Vec<String> {
    remove_dropped_package_files_with(dir, current, |path| {
        std::fs::remove_file(path)
    })
}

fn setup(
    reader: BinaryReader,
    dir: Option<PathBuf>,
    clear: bool,
    _args: &Vec<String>,
    _ui: &mut bool,
) -> Option<PathBuf> {
    let dir = if let Some(dir) = dir {
        dir
    } else if let Some(dir) = dirs::data_local_dir() {
        dir.join(app_dir_name(&reader.exe))
    } else {
        eprintln!("not found data local dir");
        return None;
    };

    let mut ts = 0;

    if clear || !is_timestamp_matches(&dir, &mut ts) {
        #[cfg(windows)]
        if _args.is_empty() {
            *_ui = true;
            ui::setup();
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    let mut metadata_paths = reader.package_paths.clone();

    metadata_paths.extend(remove_dropped_package_files(
        &dir,
        &reader.package_paths,
    ));

    for file in reader.files.iter() {
        file.write_to_file(&dir);
    }

    write_meta(&dir, ts, &metadata_paths);

    #[cfg(windows)]
    win::copy_runtime_broker(&dir);

    #[cfg(linux)]
    reader.configure_permission(&dir);

    Some(dir.join(&reader.exe))
}

fn use_null_stdio() -> bool {
    #[cfg(windows)]
    {
        let is_windows_7 = is_windows_7();
        println!("is windows7: {}", is_windows_7);
        return is_windows_7;
    }

    #[cfg(not(windows))]
    false
}

#[cfg(windows)]
fn is_windows_7() -> bool {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

    unsafe {
        let mut version_info = OSVERSIONINFOW::default();

        version_info.dwOSVersionInfoSize =
            std::mem::size_of::<OSVERSIONINFOW>() as u32;

        if RtlGetVersion(&mut version_info).is_ok() {
            println!(
                "Windows version: {}.{}",
                version_info.dwMajorVersion,
                version_info.dwMinorVersion
            );

            return version_info.dwMajorVersion == 6
                && version_info.dwMinorVersion == 1;
        }
    }

    false
}

//
// EMME EFFE - controllo servizio RustDesk
//

#[cfg(windows)]
fn is_rustdesk_service_installed() -> bool {
    use std::os::windows::process::CommandExt;

    Command::new("sc.exe")
        .args(["query", "RustDesk"])
        .creation_flags(winapi::um::winbase::CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

//
// EMME EFFE - percorso dell'eseguibile installato
//

#[cfg(windows)]
fn installed_rustdesk_exe() -> Option<PathBuf> {
    let program_files = std::env::var_os("ProgramFiles")?;
    Some(
        PathBuf::from(program_files)
            .join("RustDesk")
            .join("RustDesk.exe"),
    )
}

//
// EMME EFFE - confronto tra la build estratta e quella installata
//
// Serve a evitare il problema verificato in prova:
// GUI Emme Effe nuova + servizio Windows rimasto alla build precedente.
// Se i due eseguibili sono diversi, il servizio viene reinstallato.
//

#[cfg(windows)]
fn installed_rustdesk_matches(exe: &Path) -> bool {
    let Some(installed) = installed_rustdesk_exe() else {
        return false;
    };

    if !installed.is_file() || !exe.is_file() {
        return false;
    }

    let Ok(extracted_bytes) = std::fs::read(exe) else {
        return false;
    };

    let Ok(installed_bytes) = std::fs::read(&installed) else {
        return false;
    };

    md5::compute(extracted_bytes) == md5::compute(installed_bytes)
}

//
// EMME EFFE - avvio servizio
//

#[cfg(windows)]
fn start_rustdesk_service() {
    use std::os::windows::process::CommandExt;

    let _ = Command::new("sc.exe")
        .args(["start", "RustDesk"])
        .creation_flags(winapi::um::winbase::CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

//
// EMME EFFE - installazione / aggiornamento automatico servizio (dalla 1.6.0: con pulizia completa)
//

#[cfg(windows)]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

// -EncodedCommand vuole il testo in UTF-16LE codificato in base64: niente problemi di virgolette.
#[cfg(windows)]
fn ps_encoded(script: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(windows)]
fn script_manutenzione(exe: &Path) -> String {
    let dir_str = |d: Option<PathBuf>| d.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let roaming = dir_str(dirs::config_dir());
    let local = dir_str(dirs::data_local_dir());
    let estratto = dir_str(exe.parent().map(|p| p.to_path_buf()));

    //
    // Un'unica elevazione UAC (RunAs) esegue tutta la manutenzione.
    // Se c'e' gia' un'installazione (servizio o cartella in Program Files), PULIZIA COMPLETA:
    // - ferma il servizio e disinstalla la vecchia versione (con il suo stesso exe, se c'e');
    // - chiude i processi rimasti e cancella Program Files\RustDesk;
    // - nelle cartelle di configurazione (utente e servizio) cancella tutto TRANNE config\RustDesk.toml
    //   (ID e chiavi del PC: il cliente mantiene lo stesso ID e l'eventuale password permanente);
    // - cancella le cartelle dei vecchi tentativi (Assistenza Emme Effe, Assistenza-Emme-Effe,
    //   %LOCALAPPDATA%\rustdesk), MAI la cartella da cui stiamo lavorando adesso.
    // Poi installa la build appena estratta e avvia il servizio.
    // RUSTDESK_APPNAME resta impostato anche nel processo elevato (server e chiave Emme Effe).
    // I percorsi dell'utente vengono passati da qui: l'utente elevato potrebbe essere un altro.
    //

    format!(
        r#"$ErrorActionPreference='SilentlyContinue'
$env:{env_key}={app_name}
$nuovo={exe}
$roaming={roaming}
$local={local}
$estratto={estratto}
$pf=Join-Path $env:ProgramFiles 'RustDesk'
try {{
  $svc=Get-Service -Name 'RustDesk' -ErrorAction SilentlyContinue
  if (($null -ne $svc) -or (Test-Path -LiteralPath $pf)) {{
    Stop-Service -Name 'RustDesk' -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 700
    $vecchio=Join-Path $pf 'RustDesk.exe'
    $dis=$nuovo
    if (Test-Path -LiteralPath $vecchio) {{ $dis=$vecchio }}
    Start-Process -FilePath $dis -ArgumentList '--uninstall' -Wait -WindowStyle Hidden
    Start-Sleep -Milliseconds 700
    Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith($pf + '\', [StringComparison]::OrdinalIgnoreCase) }} | Stop-Process -Force
    if ($null -ne (Get-Service -Name 'RustDesk' -ErrorAction SilentlyContinue)) {{ sc.exe delete RustDesk | Out-Null }}
    Start-Sleep -Milliseconds 700
    Remove-Item -LiteralPath $pf -Recurse -Force
    $conf=@()
    if ($roaming) {{ $conf+=(Join-Path $roaming 'RustDesk') }}
    $conf+=(Join-Path $env:windir 'ServiceProfiles\LocalService\AppData\Roaming\RustDesk')
    $conf+=(Join-Path $env:windir 'System32\config\systemprofile\AppData\Roaming\RustDesk')
    foreach ($d in $conf) {{
      if (Test-Path -LiteralPath $d) {{
        Get-ChildItem -LiteralPath $d -Force | Where-Object {{ $_.Name -ne 'config' }} | Remove-Item -Recurse -Force
        $c=Join-Path $d 'config'
        if (Test-Path -LiteralPath $c) {{ Get-ChildItem -LiteralPath $c -Force | Where-Object {{ $_.Name -ne 'RustDesk.toml' }} | Remove-Item -Recurse -Force }}
      }}
    }}
    if ($roaming) {{
      $id=Join-Path $roaming 'RustDesk\config\RustDesk.toml'
      foreach ($n in @('Assistenza-Emme-Effe','Assistenza Emme Effe')) {{
        $v=Join-Path $roaming $n
        $t=Join-Path $v ('config\' + $n + '.toml')
        if (!(Test-Path -LiteralPath $id) -and (Test-Path -LiteralPath $t)) {{
          New-Item -ItemType Directory -Force -Path (Split-Path $id) | Out-Null
          Copy-Item -LiteralPath $t -Destination $id -Force
        }}
        if (Test-Path -LiteralPath $v) {{ Remove-Item -LiteralPath $v -Recurse -Force }}
      }}
    }}
    if ($local) {{
      Get-ChildItem -LiteralPath $local -Directory -Force | Where-Object {{
        ($_.Name -eq 'rustdesk' -or $_.Name -like 'rustdesk-host=*') -and ($_.FullName.TrimEnd('\') -ne $estratto.TrimEnd('\'))
      }} | Remove-Item -Recurse -Force
    }}
  }}
  $i=Start-Process -FilePath $nuovo -ArgumentList '--silent-install' -Wait -PassThru
  Start-Sleep -Milliseconds 700
  Start-Service -Name 'RustDesk' -ErrorAction SilentlyContinue
  exit $i.ExitCode
}} catch {{
  exit 1
}}"#,
        env_key = APPNAME_RUNTIME_ENV_KEY,
        app_name = ps_quote(EMMEEFFE_APPNAME),
        exe = ps_quote(&exe.to_string_lossy()),
        roaming = ps_quote(&roaming),
        local = ps_quote(&local),
        estratto = ps_quote(&estratto),
    )
}

#[cfg(windows)]
fn install_or_update_rustdesk_service(exe: &Path) -> bool {
    use std::os::windows::process::CommandExt;

    let interno = script_manutenzione(exe);
    let script = format!(
        "try {{ \
             $p = Start-Process -FilePath 'powershell.exe' -Verb RunAs -WindowStyle Hidden -Wait -PassThru \
                 -ArgumentList '-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {enc}'; \
             exit $p.ExitCode \
         }} catch {{ exit 1 }}",
        enc = ps_encoded(&interno),
    );

    let status = Command::new("powershell.exe")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(&script)
        .creation_flags(winapi::um::winbase::CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let install_ok = status
        .map(|status| status.success())
        .unwrap_or(false);

    if !install_ok {
        return false;
    }

    //
    // Attendiamo che Windows registri il servizio e che la copia installata
    // corrisponda davvero alla build estratta.
    //

    for _ in 0..30 {
        if is_rustdesk_service_installed()
            && installed_rustdesk_matches(exe)
        {
            start_rustdesk_service();

            std::thread::sleep(
                std::time::Duration::from_secs(2),
            );

            return true;
        }

        std::thread::sleep(
            std::time::Duration::from_millis(500),
        );
    }

    false
}

fn execute(path: PathBuf, args: Vec<String>, _ui: bool) {
    println!("executing {}", path.display());

    let mut cmd = Command::new(path);

    cmd.args(args);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        cmd.creation_flags(
            winapi::um::winbase::CREATE_NO_WINDOW,
        );

        if _ui {
            cmd.env(
                SET_FOREGROUND_WINDOW_ENV_KEY,
                "1",
            );
        }
    }

    //
    // EMME EFFE
    //
    // Il RustDesk estratto riceve sempre il nome configurato
    // con server e chiave Emme Effe.
    //
    // Il file distribuito al cliente può quindi chiamarsi:
    //
    // EmmeEffe-Assistenza.exe
    //

    cmd.env(
        APPNAME_RUNTIME_ENV_KEY,
        EMMEEFFE_APPNAME,
    );

    if use_null_stdio() {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    } else {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    }

    let _child = cmd.spawn();

    #[cfg(windows)]
    if _ui {
        match _child {
            Ok(child) => unsafe {
                winapi::um::winuser::AllowSetForegroundWindow(
                    child.id() as u32,
                );
            },

            Err(e) => {
                eprintln!("{:?}", e);
            }
        }
    }
}

fn main() -> Result<(), String> {
    let mut args = Vec::new();
    let mut arg_exe = Default::default();
    let mut i = 0;

    for arg in std::env::args() {
        if i == 0 {
            arg_exe = arg.clone();
        } else {
            args.push(arg);
        }

        i += 1;
    }

    let click_setup =
        args.is_empty()
            && arg_exe
                .to_lowercase()
                .ends_with("install.exe");

    #[cfg(windows)]
    let quick_support =
        args.is_empty()
            && win::is_quick_support_exe(&arg_exe);

    #[cfg(not(windows))]
    let quick_support = false;

    let mut ui = false;

    let reader = BinaryReader::new()?;

    if let Some(exe) = setup(
        reader,
        None,
        click_setup
            || args.contains(
                &"--silent-install".to_owned(),
            ),
        &args,
        &mut ui,
    ) {

        //
        // ======================================================
        // EMME EFFE - INSTALLAZIONE AUTOMATICA DEL SERVIZIO
        // ======================================================
        //
        // Solo nel normale avvio dell'applicazione.
        //
        // Se il servizio RustDesk non esiste:
        // 1. viene richiesta elevazione UAC;
        // 2. RustDesk viene installato con --silent-install;
        // 3. il servizio viene avviato;
        // 4. il programma prosegue normalmente.
        //
        // Se il servizio esiste già:
        // viene semplicemente richiesto l'avvio.
        //

        #[cfg(windows)]
        if args.is_empty() && !click_setup {
            let service_missing =
                !is_rustdesk_service_installed();

            let installed_build_is_old =
                !installed_rustdesk_matches(&exe);

            //
            // Se il servizio manca OPPURE l'eseguibile installato non
            // coincide con quello appena estratto, reinstalliamo.
            //
            // In questo modo la GUI e il processo --service/--server
            // lavorano sempre sulla stessa identica build.
            //

            if service_missing || installed_build_is_old {
                let _ =
                    install_or_update_rustdesk_service(&exe);
            }

            //
            // Se è già tutto allineato, nessun UAC:
            // ci limitiamo ad assicurarci che il servizio sia avviato.
            //

            if is_rustdesk_service_installed()
                && installed_rustdesk_matches(&exe)
            {
                start_rustdesk_service();

                std::thread::sleep(
                    std::time::Duration::from_secs(1),
                );
            }
        }

        if click_setup {
            args = vec![
                "--install".to_owned()
            ];
        } else if quick_support {
            args = vec![
                "--quick_support".to_owned()
            ];
        }

        execute(
            exe,
            args,
            ui,
        );
    }

    Ok(())
}

#[cfg(windows)]
mod win {
    use std::{
        fs,
        os::windows::process::CommandExt,
        path::Path,
        process::Command,
    };

    pub const RUNTIME_BROKER_EXE: &'static str =
        "C:\\Windows\\System32\\RuntimeBroker.exe";

    pub const WIN_TOPMOST_INJECTED_PROCESS_EXE: &'static str =
        "RuntimeBroker_rustdesk.exe";

    pub(super) fn copy_runtime_broker(dir: &Path) {
        let src = RUNTIME_BROKER_EXE;
        let tgt = WIN_TOPMOST_INJECTED_PROCESS_EXE;

        let target_file = dir.join(tgt);

        if target_file.exists() {
            if let (
                Ok(src_file),
                Ok(tgt_file),
            ) = (
                fs::read(src),
                fs::read(&target_file),
            ) {
                let src_md5 =
                    format!("{:x}", md5::compute(&src_file));

                let tgt_md5 =
                    format!("{:x}", md5::compute(&tgt_file));

                if src_md5 == tgt_md5 {
                    return;
                }
            }
        }

        let _allow_err =
            Command::new("taskkill")
                .args(&[
                    "/F",
                    "/IM",
                    "RuntimeBroker_rustdesk.exe",
                ])
                .creation_flags(
                    winapi::um::winbase::CREATE_NO_WINDOW,
                )
                .output();

        let _allow_err =
            std::fs::copy(
                src,
                &format!(
                    "{}\\{}",
                    dir.to_string_lossy(),
                    tgt
                ),
            );
    }

    #[inline]
    pub(super) fn is_quick_support_exe(
        exe: &str,
    ) -> bool {
        let exe = exe.to_lowercase();

        exe.contains("-qs-")
            || exe.contains("-qs.exe")
            || exe.contains("_qs.exe")
    }
}

#[cfg(test)]
mod meta_tests {
    use super::*;

    #[test]
    fn resolve_within_rejects_paths_that_escape() {
        let base = Path::new("/base");

        assert_eq!(
            resolve_within(
                base,
                "./data/logo.png",
            ),
            Some(
                base
                    .join("data")
                    .join("logo.png")
            )
        );

        assert_eq!(
            resolve_within(
                base,
                ".\\data\\logo.png",
            ),
            Some(
                base
                    .join("data")
                    .join("logo.png")
            )
        );

        assert_eq!(
            resolve_within(
                base,
                "../../etc/passwd",
            ),
            None
        );

        assert_eq!(
            resolve_within(
                base,
                "/etc/passwd",
            ),
            None
        );

        assert_eq!(
            resolve_within(
                base,
                "C:\\Windows\\System32\\x.dll",
            ),
            None
        );

        assert_eq!(
            resolve_within(
                base,
                ".",
            ),
            None
        );

        assert_eq!(
            resolve_within(
                base,
                "",
            ),
            None
        );
    }
}

#[cfg(all(test, windows))]
mod emmeeffe_tests {
    use super::*;

    #[test]
    fn ps_encoded_e_utf16le_base64() {
        // "ab" in UTF-16LE = 61 00 62 00 -> "YQBiAA=="
        assert_eq!(ps_encoded("ab"), "YQBiAA==");
        assert_eq!(ps_encoded("a"), "YQA=");
        assert_eq!(ps_encoded("abc"), "YQBiAGMA");
    }

    #[test]
    fn script_manutenzione_scritto_su_file() {
        // Con EMMEEFFE_SCRIPT_OUT impostata lo script viene salvato, per controllarlo con il parser di PowerShell.
        let s = script_manutenzione(Path::new(r"C:\Users\prova\AppData\Local\assistenza-emme-effe\Assistenza-Emme-Effe.exe"));
        assert!(s.contains("RUSTDESK_APPNAME"));
        assert!(s.contains("'RustDesk.toml'"));
        if let Ok(out) = std::env::var("EMMEEFFE_SCRIPT_OUT") {
            std::fs::write(out, &s).unwrap();
        }
    }
}
