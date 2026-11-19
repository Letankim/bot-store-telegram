# =============================================================================
# Helper Script chạy BotStoreTelegram trên Windows (Tự động ưu tiên toolchain 64-bit)
# =============================================================================
$ErrorActionPreference = "Stop"

$cleanPath = ($env:PATH -split ';' | Where-Object { $_ -notmatch 'MinGW' }) -join ';'
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:USERPROFILE\.w64devkit\w64devkit\bin;$cleanPath"

if (-not (Test-Path ".env")) {
    Write-Host "⚠️ Chưa tìm thấy file .env! Đang copy từ .env.example..." -ForegroundColor Yellow
    Copy-Item ".env.example" ".env"
    Write-Host "👉 Vui lòng mở file .env điền TELEGRAM_BOT_TOKEN, DATABASE_URL, PAYOS_* và CREDENTIALS_MASTER_KEY rồi chạy lại .\run-dev.ps1" -ForegroundColor Cyan
    exit 1
}

Write-Host "🚀 Đang khởi chạy BotStoreTelegram..." -ForegroundColor Green
cargo run --bin bot_store_telegram
