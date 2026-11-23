# FH6 Turkce Dublaj - dagitim paketi olusturur (Kurulum.exe + Veri klasoru + zip)
param([switch]$SadeceExe)
$ErrorActionPreference = 'Stop'
$kaynak = Split-Path -Parent $MyInvocation.MyCommand.Path
$kok = Split-Path -Parent $kaynak
$oyun = 'D:\STEAM\steamapps\common\ForzaHorizon6\media\Audio\FMODBanks'
$masaustu = [Environment]::GetFolderPath('Desktop')
$cikis = Join-Path $masaustu 'FH6_Turkce_Dublaj_Kurulum'

Write-Host '=== 1/3 Kurulum.exe derleniyor ==='
$csc = @("$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe", "$env:WINDIR\Microsoft.NET\Framework\v4.0.30319\csc.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $csc) { throw 'csc.exe bulunamadi (.NET Framework 4 gerekli)' }
if (-not $SadeceExe -and (Test-Path $cikis)) { Remove-Item $cikis -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $cikis 'Veri') | Out-Null
& $csc /nologo /codepage:65001 /optimize+ /target:winexe /platform:anycpu "/win32manifest:$kaynak\app.manifest" "/out:$cikis\Kurulum.exe" /r:System.Windows.Forms.dll /r:System.Drawing.dll "$kaynak\Kurulum.cs"
if ($LASTEXITCODE -ne 0) { throw 'Derleme hatasi (yukaridaki mesajlari Claude a gonderin)' }
Write-Host '   Kurulum.exe hazir.'

if ($SadeceExe) {
    Copy-Item (Join-Path $kaynak 'Oku_Beni.txt') $cikis -Force
} else {
Write-Host '=== 2/3 Turkce ses dosyalari toplaniyor (oyun klasorunden) ==='
if (-not (Test-Path $oyun)) { throw "Oyun klasoru bulunamadi: $oyun" }
$orj = @{}
Get-ChildItem (Join-Path $kok 'Orijinal_EN') -Recurse -Filter *.assets.bank | ForEach-Object { $orj[$_.Name] = $_.Length }
$n = 0; $top = 0
foreach ($ad in ($orj.Keys | Sort-Object)) {
    $f = Join-Path $oyun $ad
    if (Test-Path $f) {
        $L = (Get-Item $f).Length
        if ($L -ne $orj[$ad]) {
            Copy-Item $f (Join-Path $cikis "Veri\$ad") -Force
            $n++; $top += $L
            Write-Host ("   {0}  ({1:N0} MB)" -f $ad, ($L / 1MB))
        }
    }
}
if ($n -eq 0) { throw 'Oyun klasorunde Turkce ses dosyasi bulunamadi. Dublaj kurulu mu?' }
Copy-Item (Join-Path $kaynak 'Oku_Beni.txt') $cikis -Force
Write-Host ("   {0} ses dosyasi, toplam {1:N2} GB" -f $n, ($top / 1GB))
}

Write-Host '=== 3/3 Paylasim icin zip yapiliyor (birkac dakika surebilir) ==='
$zip = "$cikis.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Push-Location $masaustu
tar -a -c -f "FH6_Turkce_Dublaj_Kurulum.zip" "FH6_Turkce_Dublaj_Kurulum"
Pop-Location
if (Test-Path $zip) { Write-Host ("   Zip hazir: {0} ({1:N2} GB)" -f $zip, ((Get-Item $zip).Length / 1GB)) }
else { Write-Host '   Zip olusturulamadi; klasoru elle sikistirabilirsiniz.' }
Write-Host ''
Write-Host "BITTI. Masaustunde: FH6_Turkce_Dublaj_Kurulum (klasor) ve FH6_Turkce_Dublaj_Kurulum.zip"
