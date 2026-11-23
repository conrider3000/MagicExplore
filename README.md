<div align="center">
  <img src="./assets/logo.png" alt="Magic Explore Logo" width="140" />
  <h1>Magic Explore v1.0</h1>
  <p><strong>Barra de busca flutuante estilo Apple Spotlight (HIG) para Windows 10 e 11</strong></p>
  <p>
    <a href="./downloads/Magic_Explore_v1.0_Setup.exe"><strong>⬇ Baixar Instalador v1.0 (.exe)</strong></a> ·
    <a href="./downloads/Magic_Explore_Portable.exe"><strong>Executável Portátil</strong></a>
  </p>
</div>

---

## ✨ Destaques

- **Atalho Nativo `Win + Space` (`WH_KEYBOARD_LL`):** Substitui o atalho de troca de idioma do Windows mediante sua autorização (na instalação ou nas configurações) e bloqueia a abertura acidental do Menu Iniciar ao soltar a tecla `Win`.
- **Busca ao Vivo de Aplicativos:** Indexa em memória os atalhos do Menu Iniciar, Área de Trabalho e aplicativos da Microsoft Store (`Get-StartApps`) com seus ícones reais.
- **`1× Enter` vs `2× Enter` Inteligente:**
  - **Com aplicativo correspondente:** `1× Enter` abre o programa selecionado na hora.
  - **Sem aplicativo correspondente:**
    - `1× Enter` (~280 ms) → Abre o **Windows Explorer** pesquisando o termo no computador inteiro (`search-ms:`).
    - `2× Enter` (duplo Enter rápido) → Abre seu **navegador padrão** (ex.: Firefox) pesquisando direto no **Google**.
- **Fundo da Tela Opaco (Opcional):** Opção no menu de Configurações para cobrir o restante do monitor com um fundo escuro focado enquanto a barra está aberta.
- **Aviso Automático em Telas de Administrador:** Monitora via `EVENT_SYSTEM_FOREGROUND` (`0% CPU` quando parado) e exibe uma cápsula flutuante discreta estilo Apple por 3 segundos (*sem roubar o foco*) quando uma janela em modo Administrador (ex.: Gerenciador de Tarefas) pausa o atalho global.
- **Exceção Automática no Windows Defender:** O instalador NSIS oferece registrar automaticamente a pasta nas exceções do Windows Defender antes da extração.

---

## 🛠️ Stack

- **Backend Nativo:** Rust + Tauri v2 + Win32 API (`WH_KEYBOARD_LL`, `SetWinEventHook`, `IShellItemImageFactory`, `window-vibrancy` Acrylic)
- **Frontend:** HTML5 + CSS3 + JavaScript Vanilla (Zero frameworks pesados)
- **Instalador:** NSIS (`installer.nsh`)
