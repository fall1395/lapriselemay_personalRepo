# Vérification des signatures SHA-256 des modules PawnIO intégrés
# À exécuter après toute mise à jour des fichiers dans pawnio/

param()

$ErrorActionPreference = 'Stop'

# Signatures attendues (PawnIO.Modules 0.2.11)
$ExpectedSignatures = @{
    "pawnio/IntelMSR.bin" = "D6ED85D65AB17A22F813EF98207D6D537155EE2DED5976A21CB48413C9B92E5F"
    "pawnio/AMDFamily17.bin" = "DAE74615761B78BDF064DFB3E136252DDCC6FC727D88F14738D0E5800D427A91"
}

$AllOk = $true

foreach ($file in $ExpectedSignatures.Keys) {
    if (-not (Test-Path $file)) {
        Write-Error "Fichier manquant : $file"
        $AllOk = $false
        continue
    }
    
    $computed = (Get-FileHash -Path $file -Algorithm SHA256).Hash.ToUpper()
    $expected = $ExpectedSignatures[$file].ToUpper()
    
    if ($computed -ne $expected) {
        Write-Error "Signature incorrecte pour $file : attendu $expected, obtenu $computed"
        $AllOk = $false
    } else {
        Write-Host "✓ $file : signature valide" -ForegroundColor Green
    }
}

if ($AllOk) {
    Write-Host "Toutes les signatures PawnIO sont valides." -ForegroundColor Green
    exit 0
} else {
    Write-Host "Certaines signatures sont invalides." -ForegroundColor Red
    exit 1
}
