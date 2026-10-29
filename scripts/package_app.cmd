@echo off
REM ������Լ��GBK(cp936) + CRLF���Ҳ����ڱ��ļ������ chcp 65001��
REM ���ݣ�2026-10-29 ʵ�⣩��chcp 65001 + UTF-8 �� BOM ���������ᱻ cmd.exe ���ֽڴ���
REM ���У����Ķ��ֽ��ַ����ܱ����м��п���������������������ض������
REM scripts\check-batch-parse.py ����֤����UTF-8 ���ʧ�ܣ�GBK+CRLF �����ɾ����ı���ǰ��������
REM ============================================================================
REM Marguerite ����˴���ű���Windows / MSI + NSIS��
REM
REM ������˵�������������ļ�����os error 5 / Permission denied����background
REM cargo �����ᾲĬʧ�ܻ���־��������˱��ű���
REM   1) ʹ�ø���� CARGO_TARGET_DIR��D:\marg_build_target����������Ĭ�� target
REM      Ŀ¼�����ļ���ͻ��
REM   2) ÿ�ι���ǰ��ո�Ŀ¼����֤�ɾ���
REM   3) �����ڡ��ɾ������������У����������ڲ����ű������ڱ���������֤����
REM
REM �״ι���������������Ҫ��ȡ���������� Tauri �����dialog / single-instance��
REM �Լ� WebView2 ���߰�װ����offlineInstaller����cargo ����Ԥ�Ⱥ󣬿���
REM   set OFFLINE=--offline
REM �������߹�������һ����ܱ���ע����ļ�����
REM ============================================================================

setlocal

REM ---- 0. ·�� ----
set "REPO_ROOT=%~dp0.."
set "TAURI_DIR=%REPO_ROOT%\gui\src-tauri"
REM ǰ��Դ��� = �ֿ�� webui/��Vue Web ����̨���û�ָ���� UI����
set "WEBUI_SRC=%REPO_ROOT%\webui\dist"
set "WEBUI_DST=%TAURI_DIR%\webui\dist"
set "TARGET_DIR=D:\marg_build_target"

REM ---- 1. ���빹��Ŀ¼������ļ�����----
REM ��2026-10-22 �ޡ�ԭʵ�� `rmdir /s /q "%TARGET_DIR%" 2>nul` ��ɾ��ʧ��**��Ĭ�̵�**��
REM ��ע��ȴ���ơ�ÿ�ι���ǰ��ո�Ŀ¼����֤�ɾ�����ʵ�����������������Σ���
REM   Ŀ¼ûɾ�� �� tauri bundling �� bundle.resources �ǡ��ϲ����ǡ���������ؽ�
REM   �� ���ڻ�����һ��ľɹ�ϣ chunk��ʵ�� 92 �����ļ� / 19.8 MB����
REM     �� HEVC ԭ���������Ƴ�Դ����ز������ڰ��
REM ���ڣ�ɾ�����**��֤��Ĳ�����**��������ֹ��
if exist "%TARGET_DIR%" (
    echo [pkg] ��ո��빹��Ŀ¼ %TARGET_DIR% ...
    rmdir /s /q "%TARGET_DIR%" 2>nul
    if exist "%TARGET_DIR%" (
        echo [pkg][����] �޷���� %TARGET_DIR%
        echo [pkg] ����ռ���ߣ��������е� marguerite-gui.exe / ɱ��ɨ�� / ���������� / �༭����
        echo [pkg] ������������������һ��ǰ�˲���İ�������ֹ����ر�ռ�ý��̺����ԡ�
        exit /b 1
    )
)
mkdir "%TARGET_DIR%" 2>nul
if not exist "%TARGET_DIR%" (
    echo [pkg][����] �޷��������빹��Ŀ¼ %TARGET_DIR%
    exit /b 1
)

REM ---- 2. �����ɹ�����������ֹ�ɲ�����Ⱦ�����----
echo [pkg] ���� webui/dist.old* ��Ŀ¼ ...
for /d %%D in ("%REPO_ROOT%\webui\dist.old*") do (
    if exist "%%D" rmdir /s /q "%%D" 2>nul && echo   removed %%D
)

REM �����ǰ�˸�����gui\src-tauri\webui\dist��ͬ�����ܿ�汾���� ����
REM tauri bundling �� bundle.resources �ǡ��ϲ����ǡ����ɲ�������°���
for /d %%D in ("%TAURI_DIR%\webui\dist.*") do (
    if exist "%%D" rmdir /s /q "%%D" 2>nul && echo   removed %%D
)

REM ---- 3. Ƕ���ѹ����� webui/dist ----
REM     ��Դ�������� tauri.conf.json �� bundle.resources��"webui/dist"�����
REM     src-tauri�����˴��ø�Ŀ¼���¹������� gui/src-tauri/webui/dist��ȷ�����
REM     �������¿���̨���resolve_webui_dir ���������� resource_dir/webui/dist����
if not exist "%WEBUI_SRC%" (
    echo [pkg][����] δ�ҵ��ѹ����� webui/dist��%WEBUI_SRC%
    echo [pkg] ���ȹ��� webui��npm run build���ٴ����
    exit /b 1
)
REM �����ز�������2026-10-23 ���Ϊ������ǿ��λͼ������
REM   ��ʷ������ 4 �ž���Ƥ�� JPG��pets/skin-*.jpg����ȱʧ���������˻��ɾ�̬ͼ��
REM   �������Ӳ��Ҫ�����Ǵ��ڡ�����Ƥ��ϵͳ��ȫ����Ϊ canvas/3D ģ����Ⱦ
REM   ���� webui/src/components/pet/skins.ts��kind=canvas|model���б����� image Ƥ������
REM   λͼ���桸�޵�����ˮӡ�زġ�Ҫ������ ���� ��Ҫ�� JPG ֻ����ÿ�δ������ֹ��
REM   ��Ϊ���� pets/ Ŀ¼��У�����ݷǿգ���ֹ"�뿽��"����û��Ҳ����ϣ�
REM   ��������� index.html��ǰ�˲��ﱾ�壩��
set "PETS_SRC=%WEBUI_SRC%\pets"
if exist "%PETS_SRC%" (
    echo [pkg] ��⵽ pets/ Ŀ¼��λͼƤ������ѡ����У��ǿ� ...
    REM �� errorlevel ������ %VAR%������ if ���ڽ�����չ��������for ѭ���︳��ֵ������
    REM ���ӳ�չ�����壩��find /v "" ����������ʱ���� 1��
    dir /b /a-d "%PETS_SRC%" 2^>nul | find /v "" >nul
    if errorlevel 1 echo [pkg][����] pets/ Ŀ¼���ڵ�Ϊ�� ���� ��Ƥ���б�������λͼ�� 404��
) else (
    echo [pkg] �� pets/ Ŀ¼����ǰƤ����Ϊ canvas/3D ��Ⱦ������������
)
if not exist "%WEBUI_SRC%\index.html" (
    echo [pkg][����] webui/dist ȱ�� index.html��ǰ�˲��ﲻ��������
    echo [pkg] ������ִ�� npm run build��
    exit /b 1
)
echo [pkg] Ƕ�� webui/dist -^> %WEBUI_DST%
if exist "%WEBUI_DST%" rmdir /s /q "%WEBUI_DST%" 2>nul
xcopy /E /I /Y "%WEBUI_SRC%" "%WEBUI_DST%" >nul
REM �������̨�������䵽��Ƕ���ص� static/dist��Control UI ֻ�ڸ�Ŀ¼���ѹ����Ŀ���̨��
REM ȱʧʱ�����ֻ��ʾ����Դ�����á��澯��2026-10-12 ʵ��ȱ�ݣ����˴�ǿ��У�顣
REM ע�⣨2026-10-13����`gui/src-tauri/build.rs` ����**Ҳ��**�� pyengine �ؿ�֮���Զ���
REM webui/dist �������� ���� ��Ϊ build.rs �� pyengine �����������ؿ����Ҳֿ�Դ
REM pyengine/src/opensquilla/gateway/static/ û�� dist/�����Ա��ű���һ���ᱻ�����ڸ��ǡ�
REM ���ﱣ��Ϊ**ǰ������**���ڱ���ǰ�ͷ���ǰ�˲���ȱʧ�������ٳе�Ψһ�ġ���λ��ְ��
set "GW_STATIC_DIST=%TAURI_DIR%\resources\pyengine\src\opensquilla\gateway\static\dist"
if exist "%GW_STATIC_DIST%" rmdir /s /q "%GW_STATIC_DIST%" 2>nul
xcopy /E /I /Y "%WEBUI_SRC%" "%GW_STATIC_DIST%" >nul
if not exist "%GW_STATIC_DIST%\index.html" (
    echo [pkg][����] ����̨δ��λ�� resources\pyengine\...\gateway\static\dist ���� Control UI ��������
    exit /b 1
)
echo [pkg] �������̨��λ������ static/dist��index.html + pets/��.

REM ---- 3. ��λ cargo ----
where cargo >nul 2>nul
if errorlevel 1 (
    echo [pkg][����] δ�ҵ� cargo�����Ȱ�װ Rust ������ PATH��https://rustup.rs����
    exit /b 1
)

REM ---- 4. ���߿��أ�Ĭ�Ͽգ�cargo ����Ԥ�Ⱥ���� --offline��----
set "OFFLINE="

REM ---- 5. ������MSI + NSIS��WebView2 �� offlineInstaller ��Ĭ���ã�----
REM ����ǰ��ȷ��һ�Ρ�������ǰ�˸����������ڡ����� tauri bundling ����㣬
REM �� build.rs �ܵ� gui/src-tauri/webui/dist ��**���ݲ�ͬ����**������ʷ��û����������
set "PKG_WEBUI=%TARGET_DIR%\release\webui\dist"
if exist "%PKG_WEBUI%" (
    echo [pkg][����] �����ǰ�˸����Ѵ��ڣ���Ӧ�ڵ� 1 ���� target һ���������%PKG_WEBUI%
    echo [pkg] ���ᵼ���²�����ɲ����ϡ����ֶ�ɾ�������ԡ�
    exit /b 1
)

echo [pkg] ��ʼ cargo tauri build��target=%TARGET_DIR%��offline=%OFFLINE%��
cd /d "%TAURI_DIR%" || exit /b 1
set "CARGO_TARGET_DIR=%TARGET_DIR%"
cargo tauri build %OFFLINE%
if errorlevel 1 (
    echo [pkg][����] ����ʧ�ܣ�����Ϸ���־��
    exit /b 1
)

REM ---- 6. �����У�飺����ǰ�˱�����Դ�����**���ļ�һ��** ----
REM ֻ�ȶ��ļ�������ץס���ϲ����ǡ���ɵĲ�����ʵ��� 92 ������
REM ����һ���ټ�һ��˫������ֹ����һ������һ����ǡ�õ�����
if not exist "%PKG_WEBUI%" (
    echo [pkg][����] ����ǰ��Ŀ¼�����ڣ�%PKG_WEBUI%
    exit /b 1
)
set "SRC_COUNT=0"
set "PKG_COUNT=0"
for /r "%WEBUI_SRC%" %%F in (*) do @set /a "SRC_COUNT+=1" >nul
for /r "%PKG_WEBUI%" %%F in (*) do @set /a "PKG_COUNT+=1" >nul
echo [pkg] ǰ���ļ�����Դ��=%SRC_COUNT% ����=%PKG_COUNT%
if not "%SRC_COUNT%"=="%PKG_COUNT%" (
    echo [pkg][����] ����ǰ���ļ�����Դ����ﲻһ�� ���� ���Ŀ¼��������һ�������
    echo [pkg] ����tauri bundling �� bundle.resources �ǡ��ϲ����ǡ��������Ŀ��Ŀ¼��
    echo [pkg] ������ɾ�� "%PKG_WEBUI%" ���ش򣬻���� %TARGET_DIR% ȫ���ؽ���
    exit /b 1
)
echo [pkg] ǰ�˲���һ����У��ͨ����%SRC_COUNT% ���ļ�����

echo [pkg] ������ɡ�����λ�� %TARGET_DIR%\release\bundle\ �£�msi / nsis����
endlocal
