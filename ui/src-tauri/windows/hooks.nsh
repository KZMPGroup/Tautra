!macro TautraStopProcesses
  nsExec::Exec `taskkill.exe /F /T /IM tautra-ui.exe`
  Pop $R9
  nsExec::Exec `taskkill.exe /F /T /IM tautra-agent.exe`
  Pop $R9
  Sleep 700
  nsExec::Exec `taskkill.exe /F /T /IM tautra-ui.exe`
  Pop $R9
  nsExec::Exec `taskkill.exe /F /T /IM tautra-agent.exe`
  Pop $R9
  Sleep 300
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro TautraStopProcesses
  Delete "$INSTDIR\*.old*"
  RMDir /r "$INSTDIR\update"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro TautraStopProcesses
  IfSilent tautra_service_done
  ReadRegStr $R9 HKLM "SYSTEM\CurrentControlSet\Services\TautraService" "ImagePath"
  StrCmp $R9 "" tautra_service_done
  IfFileExists "$INSTDIR\tautra-agent.exe" 0 tautra_service_done
  ExecShellWait "runas" "$INSTDIR\tautra-agent.exe" "--service-uninstall" SW_HIDE
  tautra_service_done:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Tautra"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "TautraAgent"
  Delete "$INSTDIR\*.old*"
  Delete "$INSTDIR\tautra-agent.exe"
  RMDir /r "$INSTDIR\update"
  RMDir "$INSTDIR"
!macroend
