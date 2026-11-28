# Starts the title with the settings measured best on a Ryzen 5 5500 + RTX 5060 Ti 16 GB, Windows 10 (2026-11-28):
# Series S profile, DLSS Quality (1280x720 inside, 1920x1080 out), DLSS Frame Generation 2x, game time kept to the
# clock below 15 fps, submissions not joined. In the prologue's first room: about 16.5 frames rendered and 33 shown a
# second, game time 99 % of real time.
#   .\scripts\play-5500.ps1 [-Exe <the title's .exe>] [-NoFrameGen] [-PrepareOnly]
# Needs NVIDIA's Streamline files (-Streamline, or VICTORY_STREAMLINE_DIR), hardware-accelerated GPU scheduling on, and
# on Windows 10 the Agility SDK's D3D12Core (-D3D12).
param(
    [string]$Exe = 'D:\!-PS4 Jailbreaked\GTA 6\GTA6.Xbox.v1.0.2.1-CYBERLEEK\game_scarlett_master.exe',
    [string]$Streamline = 'D:\victory-data\streamline\bin-x64',
    [string]$D3D12 = 'D:\victory-leafy\target\diag\D3D12',
    [switch]$NoFrameGen,
    [switch]$PrepareOnly
)
$env:VICTORY_STREAMLINE_DIR = $Streamline
# Joined submissions (the launcher's 8) left a submission a frame or so waiting 40 to 60 ms for the card in some
# runs ("a copy at its place in a list writes what it reads"): 8.7 to 9.4 fps where it struck, 14.6 where it did not.
# Not joined, none in four runs.
$more = @{
    VICTORY_D3D12_DIR = $D3D12
    VICTORY_SIM_STEP_MAX_FPS = '10'
    VICTORY_JOIN_PARTS = '1'
    # Spare scratch chunks kept through memory pressure (the pressed 128 MB, dropped every second): 15.8 fps, not 14.0.
    VICTORY_SPARE_SCRATCH_MB = '512'
    # Four threads translate lists, not eight: fewer beside the title's own on six cores (16.6 and 16.9 fps, 15.8-16.1 with 8).
    VICTORY_THIN_THREADS = '4'
}
& (Join-Path $PSScriptRoot 'play-foxhound.ps1') -Exe $Exe -SeriesS -Dlss quality -FrameGen:(-not $NoFrameGen) -PrepareOnly:$PrepareOnly -UsedOnly:$PrepareOnly -More $more
