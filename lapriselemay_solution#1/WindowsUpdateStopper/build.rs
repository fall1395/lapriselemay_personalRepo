fn main() {
    // En release, l'exe demande les droits administrateur au lancement : sans eux, Windows refuse
    // d'arrêter ses services. En debug (ou avec WINDOWS_UPDATE_STOPPER_NO_ADMIN=1), on reste en asInvoker
    // pour tester la fenêtre sans UAC : la liste s'affiche en lecture seule.
    println!("cargo:rerun-if-env-changed=WINDOWS_UPDATE_STOPPER_NO_ADMIN");
    // Comme GameMonitor : les DLL liées à l'exe (dwmapi.dll, comctl32.dll…) sont cherchées dans
    // System32 seulement. Sinon, Windows regarde d'abord dans le dossier de l'exe : une DLL de ce
    // nom déposée à côté de Windows Update Stopper s'exécuterait, avec les droits administrateur, avant même
    // `main`. Windows 10 1607 et plus ; les versions plus anciennes ignorent l'option.
    println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    // Manifeste intégré à l'exe : la version 6 des contrôles communs de Windows (liste, cases à
    // cocher et thème sombre de l'Explorateur en dépendent), et, en release, les droits
    // administrateur.
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTDEPENDENCY:type='win32' \
         name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' \
         publicKeyToken='6595b64144ccf1df' language='*'"
    );
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let no_admin = std::env::var_os("WINDOWS_UPDATE_STOPPER_NO_ADMIN").is_some();
    if release && !no_admin {
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTUAC:level='requireAdministrator' uiAccess='false'"
        );
    }
}
