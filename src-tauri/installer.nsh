!macro NSIS_HOOK_PREINSTALL
  CreateDirectory "$INSTDIR"
  CreateDirectory "$APPDATA\Magic Explore"

  MessageBox MB_YESNO|MB_ICONINFORMATION "Configuração de Antivírus / Windows Defender:$\r$\n$\r$\nComo o Magic Explore intercepta o atalho global Win + Space nativamente e não possui certificado comercial pago da Microsoft, o Windows Defender pode acusar falso-positivo durante a instalação.$\r$\n$\r$\nDeseja adicionar automaticamente o Magic Explore nas exceções do Windows Defender agora? (Recomendado — abrirá a confirmação de Administrador)" /SD IDYES IDYES add_defender_exclusion IDNO skip_defender_exclusion

  add_defender_exclusion:
    ExecShellWait "runas" "powershell.exe" '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -Command "Add-MpPreference -ExclusionPath \"$INSTDIR\", \"$APPDATA\Magic Explore\" -ExclusionProcess \"$INSTDIR\Magic Explore.exe\" -ErrorAction SilentlyContinue"' SW_HIDE

  skip_defender_exclusion:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  CreateDirectory "$APPDATA\Magic Explore"

  ; Ativa iniciar junto com o Windows por padrão
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "MagicExplore" '"$INSTDIR\Magic Explore.exe"'

  ; Se ainda não houver configuração salva, solicita autorização para substituir Win + Space
  IfFileExists "$APPDATA\Magic Explore\config.json" skip_consent 0

  MessageBox MB_YESNO|MB_ICONQUESTION "O Magic Explore usa o atalho Win + Space para abrir a barra de busca flutuante.$\r$\n$\r$\nNo Windows, Win + Space é usado nativamente para alternar o idioma/layout do teclado (você continuará podendo trocar com Alt + Shift ou pela barra de tarefas).$\r$\n$\r$\nDeseja autorizar o Magic Explore a substituir o atalho Win + Space?" /SD IDYES IDYES authorize_winspace IDNO fallback_altspace

  authorize_winspace:
    FileOpen $0 "$APPDATA\Magic Explore\config.json" w
    FileWrite $0 '{"hotkey":{"win":true,"ctrl":false,"alt":false,"shift":false,"vk":32},"hotkey_authorized":true,"consent_asked":true,"autostart":true,"show_admin_toast":true,"opaque_backdrop":false}'
    FileClose $0
    Goto skip_consent

  fallback_altspace:
    FileOpen $0 "$APPDATA\Magic Explore\config.json" w
    FileWrite $0 '{"hotkey":{"win":false,"ctrl":false,"alt":true,"shift":false,"vk":32},"hotkey_authorized":true,"consent_asked":true,"autostart":true,"show_admin_toast":true,"opaque_backdrop":false}'
    FileClose $0

  skip_consent:
  ; Aviso final antes de fechar o instalador sobre exceção no antivírus
  MessageBox MB_YESNO|MB_ICONINFORMATION "Importante antes de finalizar a instalação:$\r$\n$\r$\nCaso você utilize outro antivírus ou o Windows Defender ainda mostre algum alerta, coloque a pasta abaixo nas Exceções do seu antivírus:$\r$\n$INSTDIR$\r$\n$\r$\nDeseja abrir o painel de Exceções do Windows Defender agora para conferir?" /SD IDNO IDYES open_defender_ui IDNO finish_postinstall

  open_defender_ui:
    ExecShell "open" "windowsdefender://exclusions"

  finish_postinstall:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "MagicExplore"
  RMDir /r "$APPDATA\Magic Explore"
!macroend
