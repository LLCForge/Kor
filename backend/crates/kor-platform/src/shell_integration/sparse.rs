//! Windows 11 sparse packages: Kor in the new top-level Explorer context menu and on the Widgets
//! board (Master Build Prompt §25.2, §27.10; ADR 0052).
//!
//! Windows 11 shows classic registry verbs only under "Show more options". The new menu takes
//! `IExplorerCommand` handlers from packaged apps, and Widgets providers must be packaged too, so
//! Kor ships two small *sparse* packages whose content stays in the install folder (external
//! location):
//! - [`MENU_PACKAGE`]: the COM surrogate entry for `shellext\kor_shellext.dll` and the
//!   `FileExplorerContextMenus` verbs for folders, folder backgrounds and files.
//! - [`WIDGETS_PACKAGE`]: `shellext\kor-widgets.exe` as COM `ExeServer` plus the
//!   `com.microsoft.windows.widgets` app extension. It depends on the Windows App Runtime
//!   ([`FRAMEWORK`]), which is why it is a separate package: the menu works without it.
//!
//! CI builds and signs the `.msix` files (`examples/sparse_package.rs`, `makeappx`, the same
//! signing as the installer, ADR 0032); `kor integrate install` registers them with
//! `Add-AppxPackage -ExternalLocation` (no administrator rights) and removes them with
//! `Remove-AppxPackage`. Every system command goes through a [`Runner`] so tests never touch the
//! machine.

use std::io;
use std::path::{Path, PathBuf};

/// Package name of the context-menu package.
pub const MENU_PACKAGE: &str = "Kor.ShellIntegration";
/// Package name of the Widgets package.
pub const WIDGETS_PACKAGE: &str = "Kor.Widgets";
/// Same value as `kor_shellext::SHELL_CLSID` (checked by a test).
pub const SHELL_CLSID: &str = "BEFBB221-CF76-40D8-87EC-2FD10BEBE592";
/// Same value as `kor_shellext::WIDGET_CLSID` (checked by a test).
pub const WIDGET_CLSID: &str = "EF27EF40-0316-45BE-907E-CAEB2E2A8D39";
/// Install sub-folder holding the DLL, the widgets exe and the two `.msix` files.
pub const DIR: &str = "shellext";
/// Windows App Runtime framework the Widgets API lives in: (package name, minimum version).
pub const FRAMEWORK: (&str, &str) = ("Microsoft.WindowsAppRuntime.1.6", "6000.519.329.0");
/// winget id that installs [`FRAMEWORK`].
pub const FRAMEWORK_WINGET: &str = "Microsoft.WindowsAppRuntime.1.6";
const MS_PUBLISHER: &str =
    "CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US";
/// First Windows build with the new context menu and Widgets board (Windows 11).
pub const MIN_BUILD: u32 = 22000;

/// Which package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Menu,
    Widgets,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Menu => MENU_PACKAGE,
            Self::Widgets => WIDGETS_PACKAGE,
        }
    }

    /// `<install>\shellext\Kor.ShellIntegration.msix` / `Kor.Widgets.msix`.
    pub fn msix(self, install_dir: &Path) -> PathBuf {
        install_dir.join(DIR).join(format!("{}.msix", self.name()))
    }
}

/// Values that differ per build.
#[derive(Debug, Clone, Copy)]
pub struct Manifest<'a> {
    /// Subject of the signing certificate (`CN=…`); must match exactly or Windows refuses it.
    pub publisher: &'a str,
    /// Product version (`0.8.0`, `0.9.0-beta.2`).
    pub version: &'a str,
    /// `x64` or `arm64`.
    pub arch: &'a str,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.to_owned())
}

/// `0.8.0` → `0.8.0.0` (package versions are four numbers ≤ 65535; pre-release tags dropped).
pub fn package_version(semver: &str) -> Option<String> {
    let core = semver.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let parts: Vec<u16> = core
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (parts.len() == 3).then(|| format!("{}.{}.{}.0", parts[0], parts[1], parts[2]))
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn header(kind: Kind, m: &Manifest<'_>) -> io::Result<String> {
    if !m.publisher.starts_with("CN=") || m.publisher.contains(['\r', '\n']) {
        return Err(invalid("publisher must be the certificate subject (CN=…)"));
    }
    if !matches!(m.arch, "x64" | "arm64") {
        return Err(invalid("arch must be x64 or arm64"));
    }
    let version = package_version(m.version).ok_or_else(|| invalid("version must be x.y.z"))?;
    let framework = match kind {
        Kind::Menu => String::new(),
        Kind::Widgets => format!(
            "\n    <PackageDependency Name=\"{}\" MinVersion=\"{}\" Publisher=\"{}\" />",
            FRAMEWORK.0,
            FRAMEWORK.1,
            xml(MS_PUBLISHER)
        ),
    };
    Ok(format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:uap3="http://schemas.microsoft.com/appx/manifest/uap/windows10/3"
  xmlns:uap10="http://schemas.microsoft.com/appx/manifest/uap/windows10/10"
  xmlns:desktop4="http://schemas.microsoft.com/appx/manifest/desktop/windows10/4"
  xmlns:desktop5="http://schemas.microsoft.com/appx/manifest/desktop/windows10/5"
  xmlns:com="http://schemas.microsoft.com/appx/manifest/com/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap uap3 uap10 desktop4 desktop5 com rescap">
  <Identity Name="{name}" Publisher="{publisher}" Version="{version}" ProcessorArchitecture="{arch}" />
  <Properties>
    <DisplayName>Kor</DisplayName>
    <PublisherDisplayName>Kor</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
    <uap10:AllowExternalContent>true</uap10:AllowExternalContent>
  </Properties>
  <Resources>
    <Resource Language="en-us" />
  </Resources>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.{min}.0" MaxVersionTested="10.0.26100.0" />{framework}
  </Dependencies>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
    <rescap:Capability Name="unvirtualizedResources" />
  </Capabilities>
"#,
        name = kind.name(),
        publisher = xml(m.publisher),
        arch = m.arch,
        min = MIN_BUILD,
    ))
}

const VISUALS: &str = r#"<uap:VisualElements AppListEntry="none" DisplayName="Kor" Description="Kor" BackgroundColor="transparent" Square150x150Logo="Assets\Square150x150Logo.png" Square44x44Logo="Assets\Square44x44Logo.png" />"#;

/// `AppxManifest.xml` of the context-menu package.
///
/// # Errors
/// On a publisher that is not a certificate subject, an unknown arch or a malformed version.
pub fn menu_manifest(m: &Manifest<'_>) -> io::Result<String> {
    let mut verbs = String::new();
    for t in ["Directory", r"Directory\Background", "*"] {
        verbs.push_str("\n            <desktop5:ItemType Type=\"");
        verbs.push_str(t);
        verbs.push_str("\"><desktop5:Verb Id=\"Kor\" Clsid=\"");
        verbs.push_str(SHELL_CLSID);
        verbs.push_str("\" /></desktop5:ItemType>");
    }
    Ok(format!(
        r#"{head}  <Applications>
    <Application Id="Kor" Executable="kor-desktop.exe" uap10:TrustLevel="mediumIL" uap10:RuntimeBehavior="win32App">
      {VISUALS}
      <Extensions>
        <desktop4:Extension Category="windows.fileExplorerContextMenus">
          <desktop4:FileExplorerContextMenus>{verbs}
          </desktop4:FileExplorerContextMenus>
        </desktop4:Extension>
        <com:Extension Category="windows.comServer">
          <com:ComServer>
            <com:SurrogateServer DisplayName="Kor context menu">
              <com:Class Id="{SHELL_CLSID}" Path="{DIR}\kor_shellext.dll" ThreadingModel="STA" />
            </com:SurrogateServer>
          </com:ComServer>
        </com:Extension>
      </Extensions>
    </Application>
  </Applications>
</Package>
"#,
        head = header(Kind::Menu, m)?,
    ))
}

/// `AppxManifest.xml` of the Widgets package.
///
/// # Errors
/// As [`menu_manifest`].
pub fn widgets_manifest(m: &Manifest<'_>) -> io::Result<String> {
    Ok(format!(
        r#"{head}  <Applications>
    <Application Id="KorWidgets" Executable="{DIR}\kor-widgets.exe" uap10:TrustLevel="mediumIL" uap10:RuntimeBehavior="win32App">
      {VISUALS}
      <Extensions>
        <com:Extension Category="windows.comServer">
          <com:ComServer>
            <com:ExeServer Executable="{DIR}\kor-widgets.exe" Arguments="-RegisterProcessAsComServer" DisplayName="Kor widgets">
              <com:Class Id="{WIDGET_CLSID}" DisplayName="Kor widget provider" />
            </com:ExeServer>
          </com:ComServer>
        </com:Extension>
        <uap3:Extension Category="windows.appExtension">
          <uap3:AppExtension Name="com.microsoft.windows.widgets" DisplayName="Kor" Id="KorWidgets" PublicFolder="Public">
            <uap3:Properties>
              <WidgetProvider>
                <ProviderIcons>
                  <Icon Path="Assets\StoreLogo.png" />
                </ProviderIcons>
                <Activation>
                  <CreateInstance ClassId="{WIDGET_CLSID}" />
                </Activation>
                <Definitions>
                  <Definition Id="kor_tasks" DisplayName="Kor" Description="Kor sessions and tasks" AllowMultiple="false">
                    <Capabilities>
                      <Capability><Size Name="small" /></Capability>
                      <Capability><Size Name="medium" /></Capability>
                      <Capability><Size Name="large" /></Capability>
                    </Capabilities>
                    <ThemeResources>
                      <Icons>
                        <Icon Path="Assets\StoreLogo.png" />
                      </Icons>
                      <Screenshots>
                        <Screenshot Path="Assets\Square150x150Logo.png" DisplayAltText="Kor" />
                      </Screenshots>
                      <DarkMode />
                      <LightMode />
                    </ThemeResources>
                  </Definition>
                </Definitions>
              </WidgetProvider>
            </uap3:Properties>
          </uap3:AppExtension>
        </uap3:Extension>
      </Extensions>
    </Application>
  </Applications>
</Package>
"#,
        head = header(Kind::Widgets, m)?,
    ))
}

/// Side-by-side manifest placed next to `kor-widgets.exe` (`kor-widgets.exe.manifest`): gives
/// the exe its package identity when COM starts it from the external location.
///
/// # Errors
/// As [`menu_manifest`].
pub fn widgets_exe_manifest(publisher: &str) -> io::Result<String> {
    if !publisher.starts_with("CN=") || publisher.contains(['\r', '\n']) {
        return Err(invalid("publisher must be the certificate subject (CN=…)"));
    }
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly manifestVersion="1.0" xmlns="urn:schemas-microsoft-com:asm.v1">
  <assemblyIdentity version="1.0.0.0" name="Kor.Widgets.Provider" />
  <msix xmlns="urn:schemas-microsoft-com:msix.v1" publisher="{}" packageName="{WIDGETS_PACKAGE}" applicationId="KorWidgets" />
</assembly>
"#,
        xml(publisher)
    ))
}

/// Write a package's staging folder: `AppxManifest.xml`, the logos (all from `logo`, a PNG) and
/// for Widgets the `Public` folder the app extension names.
///
/// # Errors
/// If a file cannot be written or `logo` cannot be read.
pub fn stage(dir: &Path, manifest_xml: &str, logo: &Path, kind: Kind) -> io::Result<()> {
    let assets = dir.join("Assets");
    std::fs::create_dir_all(&assets)?;
    std::fs::write(dir.join("AppxManifest.xml"), manifest_xml)?;
    for name in [
        "StoreLogo.png",
        "Square150x150Logo.png",
        "Square44x44Logo.png",
    ] {
        std::fs::copy(logo, assets.join(name))?;
    }
    if kind == Kind::Widgets {
        let public = dir.join("Public");
        std::fs::create_dir_all(&public)?;
        std::fs::write(public.join("kor.txt"), "Kor widgets\n")?;
    }
    Ok(())
}

/// Result of one command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a program; injected so tests never change the system.
pub trait Runner {
    /// # Errors
    /// When the program cannot be started.
    fn run(&mut self, program: &str, args: &[String]) -> io::Result<Output>;
}

/// The real system.
#[derive(Debug, Default)]
pub struct System;

impl Runner for System {
    fn run(&mut self, program: &str, args: &[String]) -> io::Result<Output> {
        use crate::NoWindow as _;
        let out = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .no_window()
            .output()?;
        Ok(Output {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// A `PowerShell` single-quoted literal (`'` doubled; nothing inside is expanded).
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn powershell(runner: &mut dyn Runner, script: &str) -> io::Result<Output> {
    runner.run(
        "powershell",
        &[
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-Command".into(),
            script.into(),
        ],
    )
}

fn checked(out: Output, what: &str) -> io::Result<Output> {
    if out.code == 0 {
        Ok(out)
    } else {
        let detail = if out.stderr.trim().is_empty() {
            out.stdout.trim()
        } else {
            out.stderr.trim()
        };
        Err(io::Error::other(format!("{what}: {detail}")))
    }
}

/// Windows build number (`22631`), from `PowerShell`.
///
/// # Errors
/// When `PowerShell` cannot run or prints no number.
pub fn windows_build(runner: &mut dyn Runner) -> io::Result<u32> {
    let out = checked(
        powershell(runner, "[Environment]::OSVersion.Version.Build")?,
        "Windows version",
    )?;
    out.stdout
        .trim()
        .parse()
        .map_err(|_| io::Error::other("Windows build number not understood"))
}

/// Windows 11 or later: the new menu and the Widgets board exist.
pub fn supported(runner: &mut dyn Runner) -> bool {
    windows_build(runner).is_ok_and(|b| b >= MIN_BUILD)
}

/// Full name of the registered package, `None` when not registered.
///
/// # Errors
/// When `PowerShell` cannot run.
pub fn status(runner: &mut dyn Runner, kind: Kind) -> io::Result<Option<String>> {
    let out = checked(
        powershell(
            runner,
            &format!(
                "(Get-AppxPackage -Name {}).PackageFullName",
                ps_quote(kind.name())
            ),
        )?,
        "Get-AppxPackage",
    )?;
    let name = out.stdout.trim();
    Ok((!name.is_empty()).then(|| name.to_owned()))
}

/// Register `kind` from `<install>\shellext\*.msix` with the install folder as external
/// location. Idempotent (`-ForceUpdateFromAnyVersion` re-registers the same version).
///
/// # Errors
/// When the package file is missing, Windows is older than Windows 11, the Windows App Runtime
/// is missing (Widgets: `NotFound`, install [`FRAMEWORK_WINGET`]) or `Add-AppxPackage` fails
/// (e.g. an untrusted signature).
pub fn register(runner: &mut dyn Runner, kind: Kind, install_dir: &Path) -> io::Result<()> {
    let msix = kind.msix(install_dir);
    if !msix.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} not found", msix.display()),
        ));
    }
    if !supported(runner) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "needs Windows 11 (build 22000 or later)",
        ));
    }
    if kind == Kind::Widgets {
        let have = checked(
            powershell(
                runner,
                &format!("(Get-AppxPackage -Name {}).Version", ps_quote(FRAMEWORK.0)),
            )?,
            "Get-AppxPackage",
        )?;
        if have.stdout.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Windows App Runtime missing: winget install {FRAMEWORK_WINGET}"),
            ));
        }
    }
    checked(
        powershell(
            runner,
            &format!(
                "Add-AppxPackage -Path {} -ExternalLocation {} -ForceUpdateFromAnyVersion",
                ps_quote(&msix.to_string_lossy()),
                ps_quote(&install_dir.to_string_lossy())
            ),
        )?,
        "Add-AppxPackage",
    )
    .map(drop)
}

/// Remove `kind`; `Ok(false)` when it was not registered.
///
/// # Errors
/// When `PowerShell` cannot run or `Remove-AppxPackage` fails.
pub fn unregister(runner: &mut dyn Runner, kind: Kind) -> io::Result<bool> {
    let Some(full) = status(runner, kind)? else {
        return Ok(false);
    };
    checked(
        powershell(
            runner,
            &format!("Remove-AppxPackage -Package {}", ps_quote(&full)),
        )?,
        "Remove-AppxPackage",
    )?;
    Ok(true)
}

/// Newest `makeappx.exe` / `signtool.exe` of an installed Windows SDK under `kits_bin`
/// (`C:\Program Files (x86)\Windows Kits\10\bin`).
pub fn sdk_tool(kits_bin: &Path, tool: &str, arch: &str) -> Option<PathBuf> {
    let mut versions: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(kits_bin)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let v: Vec<u32> = name
                .split('.')
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            let exe = e.path().join(arch).join(format!("{tool}.exe"));
            exe.is_file().then_some((v, exe))
        })
        .collect();
    versions.sort();
    versions.pop().map(|(_, p)| p)
}

/// `makeappx pack` a staged folder into `out` (no semantic validation of the external files,
/// which are not in the package).
///
/// # Errors
/// When makeappx fails.
pub fn pack(
    runner: &mut dyn Runner,
    makeappx: &Path,
    staging: &Path,
    out: &Path,
) -> io::Result<()> {
    checked(
        runner.run(
            &makeappx.to_string_lossy(),
            &[
                "pack".into(),
                "/o".into(),
                "/nv".into(),
                "/d".into(),
                staging.to_string_lossy().into_owned(),
                "/p".into(),
                out.to_string_lossy().into_owned(),
            ],
        )?,
        "makeappx",
    )
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records every command and answers from a script.
    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        build: u32,
        registered: Vec<String>,
        runtime: bool,
        fail_add: bool,
    }

    impl Runner for Fake {
        fn run(&mut self, program: &str, args: &[String]) -> io::Result<Output> {
            let line = format!("{program} {}", args.join(" "));
            self.calls.push(line.clone());
            let script = args.last().cloned().unwrap_or_default();
            let ok = |s: String| {
                Ok(Output {
                    code: 0,
                    stdout: s,
                    stderr: String::new(),
                })
            };
            if script.contains("OSVersion") {
                return ok(self.build.to_string());
            }
            if script.starts_with("(Get-AppxPackage -Name 'Microsoft.WindowsAppRuntime") {
                return ok(if self.runtime {
                    "6000.519.329.0".into()
                } else {
                    String::new()
                });
            }
            if let Some(rest) = script.strip_prefix("(Get-AppxPackage -Name '") {
                let name = rest.split('\'').next().unwrap_or_default();
                let full = self
                    .registered
                    .iter()
                    .find(|r| r.starts_with(name))
                    .cloned()
                    .unwrap_or_default();
                return ok(full);
            }
            if script.starts_with("Add-AppxPackage") {
                if self.fail_add {
                    return Ok(Output {
                        code: 1,
                        stdout: String::new(),
                        stderr: "0x800B0109 untrusted root".into(),
                    });
                }
                let name = if script.contains("Kor.Widgets.msix") {
                    WIDGETS_PACKAGE
                } else {
                    MENU_PACKAGE
                };
                if !self.registered.iter().any(|r| r.starts_with(name)) {
                    self.registered.push(format!("{name}_0.8.0.0_x64__abc"));
                }
                return ok(String::new());
            }
            if let Some(rest) = script.strip_prefix("Remove-AppxPackage -Package '") {
                let full = rest.trim_end_matches('\'');
                self.registered.retain(|r| r != full);
                return ok(String::new());
            }
            ok(String::new())
        }
    }

    const M: Manifest<'static> = Manifest {
        publisher: "CN=Kor & Co, O=Kor",
        version: "0.8.0-beta.1",
        arch: "x64",
    };

    #[test]
    fn versions_become_four_numbers() {
        assert_eq!(package_version("0.8.0").as_deref(), Some("0.8.0.0"));
        assert_eq!(
            package_version("v1.2.3-beta.4+x").as_deref(),
            Some("1.2.3.0")
        );
        assert_eq!(package_version("1.2"), None);
        assert_eq!(package_version("1.2.70000"), None);
    }

    #[test]
    fn manifests_name_the_shell_extension_and_widget_provider() {
        let menu = menu_manifest(&M).unwrap();
        assert!(menu.contains(
            r#"Name="Kor.ShellIntegration" Publisher="CN=Kor &amp; Co, O=Kor" Version="0.8.0.0""#
        ));
        for t in [
            r#"Type="Directory""#,
            r#"Type="Directory\Background""#,
            r#"Type="*""#,
        ] {
            assert!(menu.contains(t), "{t}");
        }
        assert_eq!(menu.matches(SHELL_CLSID).count(), 4);
        assert!(menu.contains(r#"Path="shellext\kor_shellext.dll" ThreadingModel="STA""#));
        assert!(menu.contains("<uap10:AllowExternalContent>true"));
        assert!(
            !menu.contains("PackageDependency"),
            "the menu must work without the App Runtime"
        );

        let w = widgets_manifest(&M).unwrap();
        assert!(w.contains(r#"Name="Kor.Widgets""#));
        assert!(w.contains(r#"<PackageDependency Name="Microsoft.WindowsAppRuntime.1.6" MinVersion="6000.519.329.0""#));
        assert!(w.contains(r#"Name="com.microsoft.windows.widgets""#));
        assert!(w.contains(r#"Arguments="-RegisterProcessAsComServer""#));
        assert_eq!(w.matches(WIDGET_CLSID).count(), 2);
        assert!(w.contains(r#"Definition Id="kor_tasks""#));

        let exe = widgets_exe_manifest(M.publisher).unwrap();
        assert!(exe.contains(r#"packageName="Kor.Widgets" applicationId="KorWidgets""#));

        assert!(
            menu_manifest(&Manifest {
                publisher: "Kor",
                ..M
            })
            .is_err()
        );
        assert!(menu_manifest(&Manifest { arch: "x86", ..M }).is_err());
        assert!(
            widgets_manifest(&Manifest {
                version: "nope",
                ..M
            })
            .is_err()
        );
        assert!(widgets_exe_manifest("CN=a\r\nb").is_err());
    }

    #[test]
    fn manifests_are_well_formed_xml() {
        // Every element opened is closed (cheap structural check; makeappx validates the schema in CI).
        for doc in [menu_manifest(&M).unwrap(), widgets_manifest(&M).unwrap()] {
            let mut stack: Vec<String> = Vec::new();
            for tag in doc.split('<').skip(1) {
                let body = tag.split('>').next().unwrap();
                if body.starts_with('?') || body.ends_with('/') {
                    continue;
                }
                if let Some(close) = body.strip_prefix('/') {
                    assert_eq!(stack.pop().as_deref(), Some(close));
                } else {
                    stack.push(body.split_whitespace().next().unwrap().to_owned());
                }
            }
            assert!(stack.is_empty(), "{stack:?}");
        }
    }

    #[test]
    fn clsids_match_the_shell_extension() {
        let src = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("shellext/src/lib.rs"),
        )
        .unwrap();
        assert!(src.contains(&format!("SHELL_CLSID: &str = \"{SHELL_CLSID}\"")));
        assert!(src.contains(&format!("WIDGET_CLSID: &str = \"{WIDGET_CLSID}\"")));
    }

    #[test]
    fn staging_writes_manifest_logos_and_public_folder() {
        let d = tempfile::tempdir().unwrap();
        let logo = d.path().join("logo.png");
        std::fs::write(&logo, b"\x89PNG").unwrap();
        let s = d.path().join("w");
        stage(&s, "<Package/>", &logo, Kind::Widgets).unwrap();
        assert_eq!(
            std::fs::read_to_string(s.join("AppxManifest.xml")).unwrap(),
            "<Package/>"
        );
        assert!(s.join("Assets/Square44x44Logo.png").is_file());
        assert!(s.join("Public/kor.txt").is_file());
        let m = d.path().join("m");
        stage(&m, "<Package/>", &logo, Kind::Menu).unwrap();
        assert!(!m.join("Public").exists());
    }

    #[test]
    fn register_status_unregister_round_trip_on_a_fake_system() {
        let d = tempfile::tempdir().unwrap();
        let install = d.path().join("Kor's app");
        std::fs::create_dir_all(install.join(DIR)).unwrap();
        let mut sys = Fake {
            build: 22631,
            ..Fake::default()
        };

        // No package file yet.
        assert_eq!(
            register(&mut sys, Kind::Menu, &install).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        std::fs::write(Kind::Menu.msix(&install), b"").unwrap();
        std::fs::write(Kind::Widgets.msix(&install), b"").unwrap();

        register(&mut sys, Kind::Menu, &install).unwrap();
        register(&mut sys, Kind::Menu, &install).unwrap();
        assert_eq!(
            status(&mut sys, Kind::Menu).unwrap().as_deref(),
            Some("Kor.ShellIntegration_0.8.0.0_x64__abc")
        );
        let add = sys
            .calls
            .iter()
            .find(|c| c.contains("Add-AppxPackage"))
            .unwrap();
        assert!(
            add.contains("-ExternalLocation '") && add.contains("Kor''s app'"),
            "{add}"
        );
        assert!(add.starts_with("powershell -NoProfile -NonInteractive"));

        // Widgets need the App Runtime first.
        let e = register(&mut sys, Kind::Widgets, &install).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert!(e.to_string().contains(FRAMEWORK_WINGET));
        sys.runtime = true;
        register(&mut sys, Kind::Widgets, &install).unwrap();
        assert!(status(&mut sys, Kind::Widgets).unwrap().is_some());

        assert!(unregister(&mut sys, Kind::Menu).unwrap());
        assert!(!unregister(&mut sys, Kind::Menu).unwrap());
        assert!(status(&mut sys, Kind::Menu).unwrap().is_none());
        assert!(status(&mut sys, Kind::Widgets).unwrap().is_some());

        // Windows 10: refused before anything is changed.
        let mut old = Fake {
            build: 19045,
            ..Fake::default()
        };
        assert_eq!(
            register(&mut old, Kind::Menu, &install).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert!(!old.calls.iter().any(|c| c.contains("Add-AppxPackage")));

        // Signature not trusted: the error text is passed on.
        let mut bad = Fake {
            build: 26100,
            fail_add: true,
            ..Fake::default()
        };
        assert!(
            register(&mut bad, Kind::Menu, &install)
                .unwrap_err()
                .to_string()
                .contains("untrusted root")
        );
    }

    #[test]
    fn sdk_tools_are_found_newest_first_and_packed() {
        let d = tempfile::tempdir().unwrap();
        for v in ["10.0.19041.0", "10.0.26100.0", "notes"] {
            let dir = d.path().join(v).join("x64");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("makeappx.exe"), b"").unwrap();
        }
        let tool = sdk_tool(d.path(), "makeappx", "x64").unwrap();
        assert!(tool.to_string_lossy().contains("10.0.26100.0"));
        assert!(sdk_tool(d.path(), "signtool", "x64").is_none());
        let mut sys = Fake::default();
        pack(&mut sys, &tool, Path::new("stage"), Path::new("out.msix")).unwrap();
        assert!(sys.calls[0].ends_with("pack /o /nv /d stage /p out.msix"));
    }
}
