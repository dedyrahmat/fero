Unicode true
Name "Fero ${VERSION}"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Fero"
RequestExecutionLevel user
Icon "Fero.ico"
UninstallIcon "Fero.ico"

!define Classes "Software\Classes"
!define Capabilities "Software\Fero\Capabilities"
!define Uninstall "Software\Microsoft\Windows\CurrentVersion\Uninstall\Fero"

Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Section "Install"
  SetOutPath "$INSTDIR\bin"
  File "${STAGE}\fero.exe"
  File "${STAGE}\*.dll"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Fero.lnk" "$INSTDIR\bin\fero.exe"
  WriteRegStr HKCU "${Uninstall}" "DisplayName" "Fero"
  WriteRegStr HKCU "${Uninstall}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${Uninstall}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${Classes}\Fero.Torrent" "" "BitTorrent metainfo"
  WriteRegStr HKCU "${Classes}\Fero.Torrent\shell\open\command" "" '"$INSTDIR\bin\fero.exe" "%1"'
  WriteRegStr HKCU "${Classes}\.torrent\OpenWithProgids" "Fero.Torrent" ""
  WriteRegStr HKCU "${Classes}\Fero.Magnet" "" "BitTorrent magnet"
  WriteRegStr HKCU "${Classes}\Fero.Magnet" "URL Protocol" ""
  WriteRegStr HKCU "${Classes}\Fero.Magnet\shell\open\command" "" '"$INSTDIR\bin\fero.exe" "%1"'
  WriteRegStr HKCU "${Capabilities}" "ApplicationName" "Fero"
  WriteRegStr HKCU "${Capabilities}" "ApplicationDescription" "BitTorrent client"
  WriteRegStr HKCU "${Capabilities}\FileAssociations" ".torrent" "Fero.Torrent"
  WriteRegStr HKCU "${Capabilities}\URLAssociations" "magnet" "Fero.Magnet"
  WriteRegStr HKCU "Software\RegisteredApplications" "Fero" "${Capabilities}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

Section "Uninstall"
  ClearErrors
  Delete "$INSTDIR\bin\fero.exe"
  IfErrors 0 +3
    MessageBox MB_OK|MB_ICONEXCLAMATION "Quit Fero from the tray before uninstalling."
    Abort
  Delete "$INSTDIR\bin\*.dll"
  RMDir "$INSTDIR\bin"
  Delete "$SMPROGRAMS\Fero.lnk"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Fero"
  DeleteRegValue HKCU "Software\RegisteredApplications" "Fero"
  DeleteRegValue HKCU "${Classes}\.torrent\OpenWithProgids" "Fero.Torrent"
  DeleteRegKey HKCU "${Classes}\Fero.Torrent"
  DeleteRegKey HKCU "${Classes}\Fero.Magnet"
  DeleteRegKey HKCU "${Capabilities}"
  DeleteRegKey HKCU "${Uninstall}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
