# Test de fumée de GameMonitor sur un vrai Windows (droits administrateur requis) :
# démarrage, session ETW des FPS, place par défaut, déverrouillage et déplacement à la souris (place
# retrouvée au lancement suivant), mode détaillé, instance unique, pause quand l'overlay est masqué,
# fermeture par la croix sans session ETW laissée ouverte.
#
#   powershell -ExecutionPolicy Bypass -File ci\smoke-test.ps1 -Exe target\release\GameMonitor.exe [-Screenshot dossier]
#
# Attention : ferme les GameMonitor déjà lancés. La place enregistrée de l'overlay est mise de côté
# pendant le test, puis remise.

param(
    [Parameter(Mandatory)] [string] $Exe,
    [string] $Screenshot
)

$ErrorActionPreference = 'Stop'
# Session ETW de GameMonitor : une par session Windows, comme celle de ce script.
$Session = "GameMonitorFPS-$((Get-Process -Id $PID).SessionId)"
$failures = [System.Collections.Generic.List[string]]::new()

function Fail([string] $message) {
    Write-Host "::error::$message"
    $failures.Add($message)
}

Add-Type -Namespace Smoke -Name Win32 -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
[DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
[DllImport("user32.dll")]
public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);
[DllImport("user32.dll")]
public static extern int GetSystemMetrics(int index);
[DllImport("user32.dll")]
public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
[StructLayout(LayoutKind.Sequential)]
public struct RECT { public int Left, Top, Right, Bottom; }
[DllImport("user32.dll")]
static extern IntPtr MonitorFromWindow(IntPtr hwnd, uint flags);
[DllImport("user32.dll")]
static extern bool GetMonitorInfoW(IntPtr monitor, ref MONITORINFO info);
[StructLayout(LayoutKind.Sequential)]
struct MONITORINFO { public int Size; public RECT Monitor, Work; public uint Flags; }
// Rectangle de l'écran qui contient la fenêtre (MONITOR_DEFAULTTONEAREST).
public static RECT ScreenOf(IntPtr hwnd) {
    var info = new MONITORINFO { Size = Marshal.SizeOf(typeof(MONITORINFO)) };
    GetMonitorInfoW(MonitorFromWindow(hwnd, 2), ref info);
    return info.Monitor;
}
[DllImport("user32.dll")]
public static extern IntPtr GetWindowLongPtrW(IntPtr hwnd, int index);
// Souris sur l'overlay, en points relatifs à son coin haut gauche. Les messages d'un même geste
// partent d'un coup : l'overlay les traite avant de regarder l'état réel du bouton (à son minuteur).
static IntPtr MakeLParam(int x, int y) { return (IntPtr) ((y << 16) | (x & 0xFFFF)); }
public static void Click(IntPtr hwnd, int x, int y) {
    PostMessageW(hwnd, 0x0201, (IntPtr) 1, MakeLParam(x, y));     // WM_LBUTTONDOWN
    PostMessageW(hwnd, 0x0202, IntPtr.Zero, MakeLParam(x, y));    // WM_LBUTTONUP
}
public static void DoubleClick(IntPtr hwnd, int x, int y) {
    Click(hwnd, x, y);
    PostMessageW(hwnd, 0x0203, (IntPtr) 1, MakeLParam(x, y));     // WM_LBUTTONDBLCLK
    PostMessageW(hwnd, 0x0202, IntPtr.Zero, MakeLParam(x, y));
}
// Glisse de (x0, y0) à (x1, y1) : l'overlay suit, le bouton est donc relâché au point saisi.
public static void Drag(IntPtr hwnd, int x0, int y0, int x1, int y1) {
    PostMessageW(hwnd, 0x0201, (IntPtr) 1, MakeLParam(x0, y0));   // WM_LBUTTONDOWN
    PostMessageW(hwnd, 0x0200, (IntPtr) 1, MakeLParam(x1, y1));   // WM_MOUSEMOVE, bouton enfoncé
    PostMessageW(hwnd, 0x0202, IntPtr.Zero, MakeLParam(x0, y0));  // WM_LBUTTONUP
}
'@

$WM_CLOSE = 0x0010
$WM_HOTKEY = 0x0312
$HK_TOGGLE = 1
$HK_MOVE = 2
$GWL_EXSTYLE = -20
$WS_EX_TRANSPARENT = 0x20
# Place de l'overlay, retenue d'un lancement à l'autre.
$Position = Join-Path $env:LOCALAPPDATA 'GameMonitor\position.txt'
# Bureau virtuel (tous les écrans) : origine, puis largeur et hauteur.
$SM_XVIRTUALSCREEN = 76
$SM_YVIRTUALSCREEN = 77
$SM_CXVIRTUALSCREEN = 78
$SM_CYVIRTUALSCREEN = 79

function Get-Overlay([int] $processId) {
    $hwnd = [IntPtr]::Zero
    # [NullString]::Value et non $null : PowerShell passerait "" (titre vide) à la place de NULL.
    while (($hwnd = [Smoke.Win32]::FindWindowExW([IntPtr]::Zero, $hwnd, 'GameMonitorOverlay', [NullString]::Value)) -ne [IntPtr]::Zero) {
        [uint32] $owner = 0
        [void] [Smoke.Win32]::GetWindowThreadProcessId($hwnd, [ref] $owner)
        if ($owner -eq $processId) { return $hwnd }
    }
    return [IntPtr]::Zero
}

# Même effet que les raccourcis Ctrl+Maj+F10 et F11 : le programme reçoit WM_HOTKEY.
function Send-Overlay([int] $processId, [uint32] $message, [int] $wParam = 0) {
    $hwnd = Get-Overlay $processId
    if ($hwnd -eq [IntPtr]::Zero) { return $false }
    return [Smoke.Win32]::PostMessageW($hwnd, $message, [IntPtr] $wParam, [IntPtr]::Zero)
}

# Clic (ou double-clic) au point (x, y) de l'overlay, relatif à son coin haut gauche.
function Send-Click([int] $processId, [int] $x, [int] $y, [switch] $Double) {
    $hwnd = Get-Overlay $processId
    if ($hwnd -eq [IntPtr]::Zero) { return }
    if ($Double) { [Smoke.Win32]::DoubleClick($hwnd, $x, $y) } else { [Smoke.Win32]::Click($hwnd, $x, $y) }
}

# Glisse l'overlay à la souris, du point (x0, y0) au point (x1, y1).
function Send-Drag([int] $processId, [int] $x0, [int] $y0, [int] $x1, [int] $y1) {
    $hwnd = Get-Overlay $processId
    if ($hwnd -ne [IntPtr]::Zero) { [Smoke.Win32]::Drag($hwnd, $x0, $y0, $x1, $y1) }
}

function Get-Rect([int] $processId) {
    $hwnd = Get-Overlay $processId
    $rect = [Smoke.Win32+RECT]::new()
    if ($hwnd -eq [IntPtr]::Zero -or -not [Smoke.Win32]::GetWindowRect($hwnd, [ref] $rect)) { return $null }
    return $rect
}

# Vrai si l'overlay laisse passer les clics jusqu'au jeu (verrouillé).
function Test-ClickThrough([int] $processId) {
    $style = [Smoke.Win32]::GetWindowLongPtrW((Get-Overlay $processId), $GWL_EXSTYLE).ToInt64()
    return ($style -band $WS_EX_TRANSPARENT) -ne 0
}

function Test-EtwSession {
    $null = logman query $Session -ets 2>&1
    return $LASTEXITCODE -eq 0
}

function Save-Screen([string] $name) {
    if (-not $Screenshot) { return }
    try {
        Add-Type -AssemblyName System.Drawing
        # Tous les écrans : l'overlay s'affiche sur celui du premier plan, pas forcément le principal.
        $originX = [Smoke.Win32]::GetSystemMetrics($SM_XVIRTUALSCREEN)
        $originY = [Smoke.Win32]::GetSystemMetrics($SM_YVIRTUALSCREEN)
        $bmp = [System.Drawing.Bitmap]::new(
            [Smoke.Win32]::GetSystemMetrics($SM_CXVIRTUALSCREEN), [Smoke.Win32]::GetSystemMetrics($SM_CYVIRTUALSCREEN))
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($originX, $originY, 0, 0, $bmp.Size)
        $g.Dispose()
        $null = New-Item -ItemType Directory -Force -Path $Screenshot
        $bmp.Save((Join-Path $Screenshot "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
    } catch {
        Write-Host "::warning::Capture d'écran impossible : $_"
    }
}

# Dans le coin demandé (0 haut gauche, 1 haut droit, 2 bas droit, 3 bas gauche), l'overlay doit tenir
# entier sur son écran, à la marge près des deux bords (10 px à 100 %, plus sur un écran mis à l'échelle).
function Test-Corner([int] $processId, [int] $corner) {
    $hwnd = Get-Overlay $processId
    $rect = [Smoke.Win32+RECT]::new()
    if ($hwnd -eq [IntPtr]::Zero -or -not [Smoke.Win32]::GetWindowRect($hwnd, [ref] $rect)) {
        Fail "coin $corner : fenêtre de l'overlay introuvable"
        return
    }
    $screen = [Smoke.Win32]::ScreenOf($hwnd)
    $inside = ($rect.Left -ge $screen.Left -and $rect.Top -ge $screen.Top -and
        $rect.Right -le $screen.Right -and $rect.Bottom -le $screen.Bottom)
    $dx = if ($corner -in 1, 2) { $screen.Right - $rect.Right } else { $rect.Left - $screen.Left }
    $dy = if ($corner -in 2, 3) { $screen.Bottom - $rect.Bottom } else { $rect.Top - $screen.Top }
    if (-not $inside -or $dx -gt 64 -or $dy -gt 64) {
        Fail ("coin {0} : overlay en ({1}, {2}, {3}, {4}), hors du coin de l'écran ({5}, {6}, {7}, {8})" -f $corner,
            $rect.Left, $rect.Top, $rect.Right, $rect.Bottom, $screen.Left, $screen.Top, $screen.Right, $screen.Bottom)
    }
}

function Stop-Overlay($process, [string] $label) {
    if ($null -eq $process -or $process.HasExited) { return }
    if (-not (Send-Overlay $process.Id $WM_CLOSE)) {
        Fail "$label : fenêtre introuvable pour la fermer proprement"
    } elseif (-not $process.WaitForExit(10000)) {
        Fail "$label : toujours lancé 10 s après WM_CLOSE"
    } elseif ($process.ExitCode -ne 0) {
        Fail "$label : code de sortie $($process.ExitCode)"
    }
}

function Start-Overlay([string] $path) {
    $process = Start-Process -FilePath $path -PassThru
    $null = $process.Handle # garde le handle pour pouvoir lire le code de sortie
    return $process
}

function Invoke-Scenario([string] $path) {
    # 1. Démarrage, sans place enregistrée : en haut à gauche, verrouillé
    $first = Start-Overlay $path
    Start-Sleep -Seconds 4
    if ($first.HasExited) {
        Fail "GameMonitor s'est arrêté au démarrage (code $($first.ExitCode))"
        return
    }
    if ((Get-Overlay $first.Id) -eq [IntPtr]::Zero) { Fail 'fenêtre de l''overlay introuvable' }
    if (-not (Test-EtwSession)) { Fail "session ETW « $Session » absente pendant l'exécution" }
    Test-Corner $first.Id 0
    if (-not (Test-ClickThrough $first.Id)) { Fail 'au démarrage, l''overlay ne laisse pas passer les clics' }
    Save-Screen 'overlay-compact'

    # 2. Ctrl+Maj+F11 le déverrouille ; un double-clic passe au mode détaillé, un autre en revient
    $null = Send-Overlay $first.Id $WM_HOTKEY $HK_MOVE
    Start-Sleep -Milliseconds 500
    if (Test-ClickThrough $first.Id) { Fail 'déverrouillé, l''overlay laisse toujours passer les clics' }
    $compact = Get-Rect $first.Id
    Send-Click $first.Id 20 20 -Double
    Start-Sleep -Milliseconds 1500
    $detailed = Get-Rect $first.Id
    if ($detailed.Bottom - $detailed.Top -le $compact.Bottom - $compact.Top) {
        Fail 'le double-clic sur l''overlay déverrouillé n''affiche pas le mode détaillé'
    }
    Save-Screen 'overlay-details'
    Send-Click $first.Id 20 20 -Double
    Start-Sleep -Milliseconds 500

    # 3. Glissé à la souris, il suit ; reverrouillé, il reste où il a été posé
    $before = Get-Rect $first.Id
    $dx = 120
    $dy = 80
    Send-Drag $first.Id 20 20 (20 + $dx) (20 + $dy)
    Start-Sleep -Milliseconds 500
    $after = Get-Rect $first.Id
    if ($after.Left - $before.Left -ne $dx -or $after.Top - $before.Top -ne $dy) {
        Fail ("glissé de ({0}, {1}), l'overlay est passé de ({2}, {3}) à ({4}, {5})" -f $dx, $dy,
            $before.Left, $before.Top, $after.Left, $after.Top)
    }
    $null = Send-Overlay $first.Id $WM_HOTKEY $HK_MOVE
    Start-Sleep -Milliseconds 1500
    if (-not (Test-ClickThrough $first.Id)) { Fail 'reverrouillé, l''overlay ne laisse pas passer les clics' }
    $locked = Get-Rect $first.Id
    if ($locked.Left -ne $after.Left -or $locked.Top -ne $after.Top) {
        Fail ("reverrouillé, l'overlay a bougé : ({0}, {1}) au lieu de ({2}, {3})" -f
            $locked.Left, $locked.Top, $after.Left, $after.Top)
    }
    Save-Screen 'overlay-deplace'

    # 4. Au lancement suivant, il retrouve sa place
    if (-not (Test-Path $Position)) { Fail "place de l'overlay non enregistrée ($Position)" }
    Stop-Overlay $first 'GameMonitor'
    $first = Start-Overlay $path
    Start-Sleep -Seconds 4
    $restored = Get-Rect $first.Id
    if ($null -eq $restored -or $restored.Left -ne $locked.Left -or $restored.Top -ne $locked.Top) {
        Fail 'au lancement suivant, l''overlay n''a pas retrouvé sa place'
    }

    # 5. Un second lancement ne doit pas créer une seconde instance
    $second = Start-Overlay $path
    if (-not $second.WaitForExit(10000)) {
        Fail 'un second lancement reste ouvert à côté du premier (pas d''instance unique)'
    }
    if ($first.HasExited) { Fail 'le premier GameMonitor s''est arrêté lors du second lancement' }
    if (-not (Test-EtwSession)) { Fail 'session ETW perdue après le second lancement' }

    # 6. Masqué (même déverrouillé) : la collecte, ETW compris, est en pause ; réaffiché, il est verrouillé
    $null = Send-Overlay $first.Id $WM_HOTKEY $HK_MOVE
    $null = Send-Overlay $first.Id $WM_HOTKEY $HK_TOGGLE
    Start-Sleep -Seconds 3
    if (Test-EtwSession) { Fail 'session ETW toujours active quand l''overlay est masqué' }
    $null = Send-Overlay $first.Id $WM_HOTKEY $HK_TOGGLE
    Start-Sleep -Seconds 3
    if (-not (Test-EtwSession)) { Fail 'session ETW non relancée quand l''overlay réapparaît' }
    if (-not (Test-ClickThrough $first.Id)) { Fail 'réaffiché, l''overlay ne laisse pas passer les clics' }

    # 7. Déverrouillé, la croix (en haut à droite) ferme GameMonitor proprement : plus de session ETW
    $null = Send-Overlay $first.Id $WM_HOTKEY $HK_MOVE
    Start-Sleep -Milliseconds 500
    $rect = Get-Rect $first.Id
    Send-Click $first.Id ($rect.Right - $rect.Left - 6) 6
    if (-not $first.WaitForExit(10000)) {
        Fail 'la croix de l''overlay déverrouillé n''a pas fermé GameMonitor'
    } elseif ($first.ExitCode -ne 0) {
        Fail "fermé par la croix : code de sortie $($first.ExitCode)"
    }
    Stop-Overlay $second 'Second lancement'
    if (Test-EtwSession) { Fail "session ETW « $Session » laissée ouverte après la fermeture" }
}

$exePath = (Resolve-Path $Exe).Path
$admin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
Write-Host "Exe : $exePath (administrateur : $admin)"

Get-Process GameMonitor -ErrorAction SilentlyContinue | Stop-Process -Force
$null = logman stop $Session -ets 2>&1
# Le test part de la place par défaut : celle enregistrée est mise de côté, puis remise.
$positionBackup = "$Position.smoke-test"
$hadPosition = Test-Path $Position
if ($hadPosition) { Move-Item $Position $positionBackup -Force }
try {
    Invoke-Scenario $exePath
} finally {
    Get-Process GameMonitor -ErrorAction SilentlyContinue | Stop-Process -Force
    $null = logman stop $Session -ets 2>&1
    Remove-Item $Position -ErrorAction SilentlyContinue
    if ($hadPosition) { Move-Item $positionBackup $Position -Force }
}

if ($failures.Count -gt 0) {
    Write-Host "$($failures.Count) vérification(s) en échec."
    exit 1
}
Write-Host 'Test de fumée réussi.'
exit 0
