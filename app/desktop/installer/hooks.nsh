; Extra steps for the Windows installer (Tauri runs these macros at the marked moments).

; After the files are copied: say what the app will fetch on first use, so nothing comes as a surprise.
!macro NSIS_HOOK_POSTINSTALL
  MessageBox MB_OK|MB_ICONINFORMATION "yt2midi is installed.$\r$\n$\r$\nThe first time you make a sheet it downloads a few helper programs, once:$\r$\n$\r$\n  - ffmpeg (about 145 MB) to read the video$\r$\n  - yt-dlp to download from YouTube$\r$\n  - Deno (small) only if you have no JavaScript runtime$\r$\n$\r$\nThey are kept in your user folder (%APPDATA%\yt2midi\bin). Nothing else leaves your computer, and your songs stay on it." /SD IDOK
!macroend

; On uninstall the songs and downloaded helpers are left alone; say where they are.
!macro NSIS_HOOK_POSTUNINSTALL
  MessageBox MB_OK|MB_ICONINFORMATION "yt2midi is removed.$\r$\n$\r$\nYour songs and the downloaded helper programs were kept in %APPDATA%\yt2midi. Delete that folder if you do not want them." /SD IDOK
!macroend
