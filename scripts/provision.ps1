# Provision a library drive (or any folder) on Windows 10/11 x64.
#
#   powershell -ExecutionPolicy Bypass -File scripts\provision.ps1 E:\           # 4B and 8B models
#   powershell -ExecutionPolicy Bypass -File scripts\provision.ps1 E:\ -Models 4b
#
# Same layout as scripts/provision.sh; see spec/ssd-layout.md.
param(
  [Parameter(Mandatory = $true)][string]$Root,
  [ValidateSet("4b", "8b", "both", "none")][string]$Models = "both"
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"   # Invoke-WebRequest is 10x slower with the progress bar
$Repo = if ($env:KB_REPO) { $env:KB_REPO } else { "benat10e9/offline" }
$Tag = "b11747"
$Plat = "windows-x86_64"

foreach ($d in "bin", "models", "packages", "trust", "private", "updates", "LICENSES") {
  New-Item -ItemType Directory -Force -Path (Join-Path $Root $d) | Out-Null
}
$Root = (Resolve-Path $Root).Path
$Tmp = Join-Path ([IO.Path]::GetTempPath()) ("kbprov-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Tmp | Out-Null

function Say($m) { Write-Host "`n== $m" }
function Fetch($Url, $Dest) { Invoke-WebRequest -Uri $Url -OutFile $Dest -UseBasicParsing }
function Sha($Path) { (Get-FileHash -Algorithm SHA256 $Path).Hash.ToLower() }
function ReleaseAsset($Tag, $Name, $Dest) {
  if ((Get-Command gh -ErrorAction SilentlyContinue) -and (gh auth status 2>$null; $LASTEXITCODE -eq 0)) {
    gh release download $Tag --repo $Repo --pattern $Name --output $Dest --clobber
    return ($LASTEXITCODE -eq 0)
  }
  try { Fetch "https://github.com/$Repo/releases/download/$Tag/$Name" $Dest; return $true } catch { return $false }
}

Say "AI engine: llama.cpp $Tag"
$builds = @(
  @{ backend = "vulkan"; asset = "llama-$Tag-bin-win-vulkan-x64.zip"; sha = "d298dc743c1e6b73b441742efedc40667232b77c0da5484aaf54475470d76198" },
  @{ backend = "cpu";    asset = "llama-$Tag-bin-win-cpu-x64.zip";    sha = "5d9f9ab6406bcc6560d60cdaf94464fc3ae661ef2e4b35cdedad3a048f1cf0bf" }
)
foreach ($b in $builds) {
  $dest = Join-Path $Root "bin\$Plat\$($b.backend)"
  if (Test-Path (Join-Path $dest "llama-server.exe")) { Write-Host "  $($b.backend): already installed"; continue }
  $zip = Join-Path $Tmp $b.asset
  Fetch "https://github.com/ggml-org/llama.cpp/releases/download/$Tag/$($b.asset)" $zip
  $got = Sha $zip
  if ($got -ne $b.sha) { throw "CHECKSUM MISMATCH for $($b.asset): $got" }
  $x = Join-Path $Tmp ("x-" + $b.backend)
  Expand-Archive -Path $zip -DestinationPath $x -Force
  $server = Get-ChildItem -Path $x -Recurse -Filter llama-server.exe | Select-Object -First 1
  New-Item -ItemType Directory -Force -Path $dest | Out-Null
  Copy-Item -Path (Join-Path $server.DirectoryName "*") -Destination $dest -Recurse -Force
  Write-Host "  $($b.backend): ok"
}

$entries = @()
function AddModel($Id, $File, $Url, $Ram) {
  $path = Join-Path $Root "models\$File"
  if (-not (Test-Path $path)) {
    Say "Model: $Id (this is the big download)"
    Fetch $Url "$path.part"
    Move-Item "$path.part" $path -Force
  } else { Say "Model: $Id already present" }
  $script:entries += [ordered]@{ id = $Id; file = $File; min_ram_gb = $Ram; context = 8192; license = "Apache-2.0"; safety_approved = $false; sha256 = (Sha $path) }
}
$HF = "https://huggingface.co"
if ($Models -in "4b", "both") { AddModel "qwen3-4b-q4km" "Qwen3-4B-Q4_K_M.gguf" "$HF/Qwen/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf" 12 }
if ($Models -in "8b", "both") { AddModel "qwen3-8b-q4km" "Qwen3-8B-Q4_K_M.gguf" "$HF/Qwen/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf" 16 }
if ($entries.Count -gt 0) {
  @{ models = $entries } | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8 (Join-Path $Root "models\models.json")
}

Say "Content packages (content-dev release)"
$pk = Join-Path $Tmp "packages.tar.gz"
if (ReleaseAsset "content-dev" "packages.tar.gz" $pk) {
  tar -xzf $pk -C (Join-Path $Root "packages")
} else { Write-Host "  not published yet; run the 'content' workflow, then re-run" }
if (-not (ReleaseAsset "content-dev" "dev-publisher.pub" (Join-Path $Root "trust\dev-publisher.pub"))) {
  Write-Host "  publisher key not published yet"
}

Say "App (app-dev release)"
$az = Join-Path $Tmp "app.zip"
if (ReleaseAsset "app-dev" "offline-library-windows-x86_64.zip" $az) {
  Expand-Archive -Path $az -DestinationPath (Join-Path $Tmp "app") -Force
  Copy-Item -Path (Join-Path $Tmp "app\Offline Library.exe") -Destination $Root -Force -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force -Path (Join-Path $Root "bin\$Plat") | Out-Null
  Copy-Item -Path (Join-Path $Tmp "app\kb.exe") -Destination (Join-Path $Root "bin\$Plat\kb.exe") -Force -ErrorAction SilentlyContinue
} else { Write-Host "  not published yet; run the 'app' workflow, then re-run" }

New-Item -ItemType File -Force -Path (Join-Path $Root "DEV_ALLOW_UNEVALUATED") | Out-Null
Remove-Item -Recurse -Force $Tmp

Say "Done: $Root"
Write-Host "Open:      `"$Root\Offline Library.exe`""
Write-Host "Benchmark: `"$Root\bin\$Plat\kb.exe`" bench --root `"$Root`" --allow-unevaluated   (send back bench-report.json)"
