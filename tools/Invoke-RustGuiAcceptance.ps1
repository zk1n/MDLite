[CmdletBinding()]
param(
    [switch]$UiParity,
    [switch]$StartupSmoke,
    [switch]$EditorRecovery,
    [string]$ExpectedExecutablePath,
    [string]$ExpectedExecutableSha256
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$ProjectRoot = Split-Path -Parent $PSScriptRoot
$DefaultExecutablePath = Join-Path $ProjectRoot 'target\release\mdlite.exe'
$ExecutablePath = if ([string]::IsNullOrWhiteSpace($ExpectedExecutablePath)) {
    $DefaultExecutablePath
} else {
    [IO.Path]::GetFullPath($ExpectedExecutablePath)
}
$RunRoot = Join-Path $ProjectRoot '.codex-runtime\editor-acceptance'
$TimeoutMs = 7000
$StartupWindowTimeoutMs = 30000
# Diagnostic refresh measured 1.9–2.3s; a 1s request can time out just before its fresh snapshot lands.
$DiagnosticMessageTimeoutMs = 3000
$script:ImeEscapesSentForComposition = 0
$DiagnosticRefreshMessage = 0x8001
$WM_CLOSE = 0x0010
$SMTO_ABORTIFHUNG = 0x0002

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class MdliteRustAcceptanceNative
{
    public static string LastSaveAsFilenameFocusFailure = String.Empty;
    private const uint INPUT_KEYBOARD = 1;
    private const uint INPUT_MOUSE = 0;
    private const uint KEYEVENTF_KEYUP = 0x0002;
    private const uint KEYEVENTF_UNICODE = 0x0004;
    private const uint MOUSEEVENTF_MOVE = 0x0001;
    private const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
    private const uint MOUSEEVENTF_LEFTUP = 0x0004;
    private const uint MOUSEEVENTF_VIRTUALDESK = 0x4000;
    private const uint MOUSEEVENTF_ABSOLUTE = 0x8000;
    private const int SM_XVIRTUALSCREEN = 76;
    private const int SM_YVIRTUALSCREEN = 77;
    private const int SM_CXVIRTUALSCREEN = 78;
    private const int SM_CYVIRTUALSCREEN = 79;

    [StructLayout(LayoutKind.Sequential)]
    private struct KEYBDINPUT
    {
        public ushort wVk;
        public ushort wScan;
        public uint dwFlags;
        public uint time;
        public UIntPtr dwExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MOUSEINPUT
    {
        public int dx;
        public int dy;
        public uint mouseData;
        public uint dwFlags;
        public uint time;
        public UIntPtr dwExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct HARDWAREINPUT
    {
        public uint uMsg;
        public ushort wParamL;
        public ushort wParamH;
    }

    [StructLayout(LayoutKind.Explicit)]
    private struct INPUTUNION
    {
        [FieldOffset(0)] public MOUSEINPUT mi;
        [FieldOffset(0)] public KEYBDINPUT ki;
        [FieldOffset(0)] public HARDWAREINPUT hi;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct INPUT
    {
        public uint type;
        public INPUTUNION data;
    }

    public static int AssertInputLayout()
    {
        int expected = IntPtr.Size == 8 ? 40 : 28;
        int actual = Marshal.SizeOf(typeof(INPUT));
        if (actual != expected)
            throw new InvalidOperationException(
                "Win32 INPUT layout mismatch: expected " + expected + " bytes, got " + actual + ".");
        return actual;
    }

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr window);

    [DllImport("user32.dll")]
    public static extern bool IsWindow(IntPtr window);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);

    [DllImport("user32.dll")]
    public static extern bool IsWindowEnabled(IntPtr window);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool GetWindowRect(IntPtr window, out RECT rect);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool GetClientRect(IntPtr window, out RECT rect);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool ClientToScreen(IntPtr window, ref POINT point);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetCursorPos(int x, int y);

    [DllImport("user32.dll")]
    private static extern int GetDlgCtrlID(IntPtr window);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool EnumChildWindows(IntPtr parent, EnumWindowProc callback, IntPtr state);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool EnumThreadWindows(uint threadId, EnumWindowProc callback, IntPtr state);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetDpiForWindow(IntPtr window);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll")]
    private static extern IntPtr GetKeyboardLayout(uint threadId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern short VkKeyScanExW(char character, IntPtr keyboardLayout);

    [DllImport("user32.dll")]
    private static extern short GetKeyState(int virtualKey);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int GetClassNameW(IntPtr window, StringBuilder className, int capacity);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int GetWindowTextW(IntPtr window, StringBuilder title, int capacity);

    [DllImport("user32.dll")]
    private static extern IntPtr GetWindow(IntPtr window, uint command);

    [DllImport("user32.dll")]
    public static extern IntPtr GetParent(IntPtr window);

    [DllImport("user32.dll")]
    private static extern bool IsChild(IntPtr parent, IntPtr window);

    [DllImport("user32.dll", ExactSpelling = true)]
    private static extern int GetWindowLongW(IntPtr window, int index);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool GetGUIThreadInfo(uint threadId, ref GUITHREADINFO info);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern int GetSystemMetrics(int index);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool GetCursorPos(out POINT point);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern short GetAsyncKeyState(int virtualKey);

    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", ExactSpelling = true, SetLastError = true)]
    private static extern IntPtr SendControlTextTimeoutW(
        IntPtr window, uint message, IntPtr wParam, IntPtr lParam,
        uint flags, uint timeoutMs, out IntPtr result);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SendMessageTimeoutW(
        IntPtr window, uint message, IntPtr wParam, IntPtr lParam,
        uint flags, uint timeoutMs, out IntPtr result);

    private delegate bool EnumWindowProc(IntPtr window, IntPtr state);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct GUITHREADINFO
    {
        public uint cbSize;
        public uint flags;
        public IntPtr hwndActive;
        public IntPtr hwndFocus;
        public IntPtr hwndCapture;
        public IntPtr hwndMenuOwner;
        public IntPtr hwndMoveSize;
        public IntPtr hwndCaret;
        public RECT rcCaret;
    }

    public sealed class EditorNativeFocusInfo
    {
        public string expected_hwnd { get; set; }
        public string window_class { get; set; }
        public bool registered_editor_window { get; set; }
        public bool gui_thread_info_available { get; set; }
        public bool foreground_is_expected { get; set; }
        public bool active_is_expected { get; set; }
        public bool focus_is_expected { get; set; }
        public bool caret_is_expected { get; set; }
        public string active_hwnd { get; set; }
        public string focus_hwnd { get; set; }
        public string caret_hwnd { get; set; }
        public int[] caret_rect { get; set; }
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct POINT
    {
        public int X;
        public int Y;
    }

    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint count, INPUT[] inputs, int inputSize);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateWindowExW(
        uint exStyle, string className, string windowName, uint style,
        int x, int y, int width, int height, IntPtr parent, IntPtr menu,
        IntPtr instance, IntPtr parameter);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool DestroyWindow(IntPtr window);

    public static IntPtr CreateRecoveryFocusWindow()
    {
        return CreateWindowExW(0, "STATIC", "MDLite Editor Recovery focus fixture",
            0x10CF0000, 40, 40, 480, 260, IntPtr.Zero, IntPtr.Zero,
            IntPtr.Zero, IntPtr.Zero);
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct TF_INPUTPROCESSORPROFILE
    {
        public uint dwProfileType;
        public ushort langid;
        public Guid clsid;
        public Guid guidProfile;
        public Guid catid;
        public IntPtr hklSubstitute;
        public uint dwCaps;
        public IntPtr hkl;
        public uint dwFlags;
    }

    public sealed class TsfProfileInfo
    {
        public uint profile_type { get; set; }
        public string profile_type_name { get; set; }
        public ushort language_id_value { get; set; }
        public string language_id { get; set; }
        public string clsid { get; set; }
        public string profile_guid { get; set; }
        public string category_guid { get; set; }
        public string description { get; set; }
        public string hkl { get; set; }
        public bool active { get; set; }
        public bool enabled { get; set; }
    }

    [ComImport, Guid("71C6E74C-0F28-11D8-A82A-00065B84435C"),
     InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface ITfInputProcessorProfileMgr
    {
        [PreserveSig] int ActivateProfile(uint type, ushort langid, ref Guid clsid, ref Guid profile, IntPtr hkl, uint flags);
        [PreserveSig] int DeactivateProfile(uint type, ushort langid, ref Guid clsid, ref Guid profile, IntPtr hkl, uint flags);
        [PreserveSig] int GetProfile(uint type, ushort langid, ref Guid clsid, ref Guid profile, IntPtr hkl, out TF_INPUTPROCESSORPROFILE value);
        [PreserveSig] int EnumProfiles(ushort langid, [MarshalAs(UnmanagedType.Interface)] out IEnumTfInputProcessorProfiles profiles);
        [PreserveSig] int ReleaseInputProcessor(ref Guid clsid, uint flags);
        [PreserveSig] int RegisterProfile(IntPtr a, IntPtr b, IntPtr c, IntPtr d, IntPtr e, IntPtr f, IntPtr g, IntPtr h, IntPtr i, IntPtr j, IntPtr k, IntPtr l);
        [PreserveSig] int UnregisterProfile(IntPtr a, IntPtr b, IntPtr c, IntPtr d);
        [PreserveSig] int GetActiveProfile(ref Guid category, out TF_INPUTPROCESSORPROFILE value);
    }

    [ComImport, Guid("71C6E74D-0F28-11D8-A82A-00065B84435C"),
     InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IEnumTfInputProcessorProfiles
    {
        [PreserveSig] int Clone([MarshalAs(UnmanagedType.Interface)] out IEnumTfInputProcessorProfiles value);
        [PreserveSig] int Next(uint count,
            [Out, MarshalAs(UnmanagedType.LPArray, SizeParamIndex = 0)] TF_INPUTPROCESSORPROFILE[] values,
            out uint fetched);
        [PreserveSig] int Reset();
        [PreserveSig] int Skip(uint count);
    }

    [ComImport, Guid("1F02B6C5-7842-4EE6-8A0B-9A24183A95CA"),
     InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface ITfInputProcessorProfiles
    {
        [PreserveSig] int Register(ref Guid clsid);
        [PreserveSig] int Unregister(ref Guid clsid);
        [PreserveSig] int AddLanguageProfile(ref Guid clsid, ushort langid, ref Guid profile, IntPtr description, uint descriptionLength, IntPtr iconFile, uint iconLength, uint iconIndex);
        [PreserveSig] int RemoveLanguageProfile(ref Guid clsid, ushort langid, ref Guid profile);
        [PreserveSig] int EnumInputProcessorInfo(out IntPtr enumerator);
        [PreserveSig] int GetDefaultLanguageProfile(ushort langid, ref Guid category, out Guid clsid, out Guid profile);
        [PreserveSig] int SetDefaultLanguageProfile(ushort langid, ref Guid clsid, ref Guid profile);
        [PreserveSig] int ActivateLanguageProfile(ref Guid clsid, ushort langid, ref Guid profile);
        [PreserveSig] int GetActiveLanguageProfile(ref Guid clsid, out ushort langid, out Guid profile);
        [PreserveSig] int GetLanguageProfileDescription(ref Guid clsid, ushort langid, ref Guid profile,
            [MarshalAs(UnmanagedType.BStr)] out string description);
    }

    public static TsfProfileInfo[] EnumerateJapaneseTextProfiles()
    {
        object instance = null;
        IEnumTfInputProcessorProfiles enumerator = null;
        List<TsfProfileInfo> found = new List<TsfProfileInfo>();
        try
        {
            Type coclass = Type.GetTypeFromCLSID(new Guid("33C53A50-F456-4884-B049-85FD643ECFED"), true);
            instance = Activator.CreateInstance(coclass);
            ITfInputProcessorProfileMgr manager = (ITfInputProcessorProfileMgr)instance;
            ITfInputProcessorProfiles profiles = (ITfInputProcessorProfiles)instance;
            int hr = manager.EnumProfiles(0x0411, out enumerator);
            if (hr < 0) Marshal.ThrowExceptionForHR(hr);
            while (true)
            {
                TF_INPUTPROCESSORPROFILE[] values = new TF_INPUTPROCESSORPROFILE[1];
                uint fetched;
                hr = enumerator.Next(1, values, out fetched);
                if (hr == 1 || fetched == 0) break;
                if (hr < 0) Marshal.ThrowExceptionForHR(hr);
                TF_INPUTPROCESSORPROFILE value = values[0];
                if (value.dwProfileType != 1 || value.langid != 0x0411 ||
                    value.catid != new Guid("34745C63-B2F0-4784-8B67-5E12C8701A31")) continue;
                string description = String.Empty;
                int descriptionHr = profiles.GetLanguageProfileDescription(
                    ref value.clsid, value.langid, ref value.guidProfile, out description);
                if (descriptionHr < 0) description = String.Empty;
                found.Add(new TsfProfileInfo {
                    profile_type = value.dwProfileType,
                    profile_type_name = "text_service",
                    language_id_value = value.langid,
                    language_id = "0x" + value.langid.ToString("X4"),
                    clsid = value.clsid.ToString("B"),
                    profile_guid = value.guidProfile.ToString("B"),
                    category_guid = value.catid.ToString("B"),
                    description = description,
                    hkl = "0x" + value.hkl.ToInt64().ToString("X"),
                    active = (value.dwFlags & 1) != 0,
                    enabled = (value.dwFlags & 2) != 0
                });
            }
            return found.ToArray();
        }
        finally
        {
            if (enumerator != null && Marshal.IsComObject(enumerator)) Marshal.ReleaseComObject(enumerator);
            if (instance != null && Marshal.IsComObject(instance)) Marshal.FinalReleaseComObject(instance);
        }
    }

    public static int ActivateInputProfile(uint profileType, ushort langid, string clsidText,
        string profileText, string hklText)
    {
        object instance = null;
        try
        {
            Type coclass = Type.GetTypeFromCLSID(new Guid("33C53A50-F456-4884-B049-85FD643ECFED"), true);
            instance = Activator.CreateInstance(coclass);
            ITfInputProcessorProfileMgr manager = (ITfInputProcessorProfileMgr)instance;
            Guid clsid = String.IsNullOrWhiteSpace(clsidText) ? Guid.Empty : Guid.Parse(clsidText);
            Guid profile = String.IsNullOrWhiteSpace(profileText) ? Guid.Empty : Guid.Parse(profileText);
            string hklValue = String.IsNullOrWhiteSpace(hklText) ? String.Empty : hklText.Trim();
            if (hklValue.StartsWith("0x", StringComparison.OrdinalIgnoreCase)) hklValue = hklValue.Substring(2);
            IntPtr hkl = profileType == 1 || String.IsNullOrWhiteSpace(hklValue)
                ? IntPtr.Zero
                : new IntPtr(Convert.ToInt64(hklValue, 16));
            return manager.ActivateProfile(profileType, langid, ref clsid, ref profile, hkl, 0x20000000);
        }
        finally
        {
            if (instance != null && Marshal.IsComObject(instance)) Marshal.FinalReleaseComObject(instance);
        }
    }

    public static string WindowClass(IntPtr window)
    {
        StringBuilder value = new StringBuilder(256);
        GetClassNameW(window, value, value.Capacity);
        return value.ToString();
    }

    public static string WindowTitle(IntPtr window)
    {
        StringBuilder value = new StringBuilder(512);
        GetWindowTextW(window, value, value.Capacity);
        return value.ToString();
    }

    // GetWindowText cannot read another process's Edit contents. WM_GETTEXT is a
    // system message, so Windows marshals this bounded UTF16 buffer across processes.
    public static string OwnedControlText(IntPtr window, uint expectedProcessId)
    {
        uint actualProcessId;
        uint threadId = GetWindowThreadProcessId(window, out actualProcessId);
        if (threadId == 0 || actualProcessId != expectedProcessId ||
            !IsOwnedWindow(window, expectedProcessId) ||
            expectedProcessId == (uint)System.Diagnostics.Process.GetCurrentProcess().Id)
            throw new InvalidOperationException("Control text requires an exact owned cross-process HWND.");
        const int capacity = 32768;
        IntPtr memory = Marshal.AllocHGlobal(capacity * 2);
        try
        {
            Marshal.Copy(new byte[capacity * 2], 0, memory, capacity * 2);
            IntPtr result;
            // BLOCK | ABORTIFHUNG | ERRORONEXIT; no NOTIMEOUTIFNOTHUNG extension.
            if (SendControlTextTimeoutW(window, 0x000D, new IntPtr(capacity), memory,
                    0x0023, 1000, out result) == IntPtr.Zero)
                throw new TimeoutException("Owned WM_GETTEXT did not complete within its bounded request.");
            uint responseProcessId;
            uint responseThreadId = GetWindowThreadProcessId(window, out responseProcessId);
            if (responseThreadId != threadId || responseProcessId != expectedProcessId ||
                !IsOwnedWindow(window, expectedProcessId))
                throw new InvalidOperationException("Control text response ownership changed.");
            long copied = result.ToInt64();
            if (copied < 0 || copied >= capacity - 1 ||
                Marshal.ReadInt16(memory, checked((int)copied * 2)) != 0)
                throw new InvalidOperationException("Control text response was invalid or reached the buffer limit.");
            string value = Marshal.PtrToStringUni(memory, (int)copied);
            if (value.IndexOf('\0') >= 0)
                throw new InvalidOperationException("Control text response contained an unexpected embedded terminator.");
            return value;
        }
        finally { Marshal.FreeHGlobal(memory); }
    }

    public static bool IsOwnedSaveAsDialog(IntPtr dialog, IntPtr main, uint expectedProcessId)
    {
        return IsOwnedWindow(main, expectedProcessId) && IsOwnedWindow(dialog, expectedProcessId) &&
            GetWindow(dialog, 4) == main && WindowClass(dialog) == "#32770" &&
            WindowTitle(dialog) == "名前を付けて保存" && IsWindowVisible(dialog) && IsWindowEnabled(dialog);
    }

    // Filename fixture setup only: a completed WM_SETTEXT avoids interpreting
    // SendInput insertion as consumption of an entire queued path.
    public static void SetOwnedSaveAsFilename(
        IntPtr dialog, IntPtr main, IntPtr edit, uint expectedProcessId, string path)
    {
        if (String.IsNullOrEmpty(path) || path.Length >= 32767 || path.IndexOf('\0') >= 0)
            throw new ArgumentException("SaveAs filename must fit the bounded non-NUL UTF16 text buffer.");
        if (GetSaveAsFilenameFocus(dialog, main, expectedProcessId) != edit ||
            (GetWindowLongW(edit, -16) & 0x0004) != 0 ||
            expectedProcessId == (uint)System.Diagnostics.Process.GetCurrentProcess().Id)
            throw new InvalidOperationException("Filename setup requires the exact owned focused single-line SaveAs Edit.");
        uint requestProcessId;
        uint requestThreadId = GetWindowThreadProcessId(edit, out requestProcessId);
        if (requestThreadId == 0 || requestProcessId != expectedProcessId)
            throw new InvalidOperationException("Filename setup request ownership changed.");
        IntPtr text = Marshal.StringToHGlobalUni(path);
        try
        {
            IntPtr result;
            if (SendControlTextTimeoutW(edit, 0x000C, IntPtr.Zero, text, 0x0023, 1000, out result) == IntPtr.Zero)
                throw new TimeoutException("Owned filename WM_SETTEXT did not complete within its bounded request.");
            uint responseProcessId;
            if (result != new IntPtr(1) ||
                GetWindowThreadProcessId(edit, out responseProcessId) != requestThreadId ||
                responseProcessId != expectedProcessId ||
                GetSaveAsFilenameFocus(dialog, main, expectedProcessId) != edit)
                throw new InvalidOperationException("Filename WM_SETTEXT failed or its response ownership/focus changed.");
        }
        finally { Marshal.FreeHGlobal(text); }
    }

    // Called after the observed File name (N) mnemonic, not to infer filename
    // semantics from an anonymous Edit ID. Keep the active/focus/caret identity exact.
    public static IntPtr GetSaveAsFilenameFocus(IntPtr dialog, IntPtr main, uint expectedProcessId)
    {
        LastSaveAsFilenameFocusFailure = String.Empty;
        if (!IsOwnedSaveAsDialog(dialog, main, expectedProcessId))
        {
            LastSaveAsFilenameFocusFailure = "OWNED_DIALOG_GATE_FAILED";
            return IntPtr.Zero;
        }
        IntPtr foreground = GetForegroundWindow();
        if (foreground != dialog)
        {
            LastSaveAsFilenameFocusFailure = "FOREGROUND_DIALOG_MISMATCH: foreground_hwnd=" +
                WindowHandleText(foreground) + "; dialog_hwnd=" + WindowHandleText(dialog);
            return IntPtr.Zero;
        }
        uint actualProcessId;
        uint threadId = GetWindowThreadProcessId(dialog, out actualProcessId);
        if (threadId == 0 || actualProcessId != expectedProcessId)
        {
            LastSaveAsFilenameFocusFailure = "DIALOG_OWNER_GATE_FAILED: thread_id=" +
                threadId + "; process_id=" + actualProcessId + "; expected_process_id=" + expectedProcessId;
            return IntPtr.Zero;
        }
        GUITHREADINFO info = new GUITHREADINFO();
        info.cbSize = (uint)Marshal.SizeOf(info);
        bool guiThreadInfoAvailable = GetGUIThreadInfo(threadId, ref info);
        if (!guiThreadInfoAvailable || info.hwndActive != dialog ||
            info.hwndFocus == IntPtr.Zero || info.hwndCaret != info.hwndFocus)
        {
            LastSaveAsFilenameFocusFailure = "GUI_THREAD_STATE_GATE_FAILED: available=" +
                guiThreadInfoAvailable + "; active_hwnd=" + WindowHandleText(info.hwndActive) +
                "; focus_hwnd=" + WindowHandleText(info.hwndFocus) +
                "; caret_hwnd=" + WindowHandleText(info.hwndCaret);
            return IntPtr.Zero;
        }
        IntPtr edit = info.hwndFocus;
        IntPtr combo = GetParent(edit);
        uint editProcessId;
        uint comboProcessId;
        if (GetWindowThreadProcessId(edit, out editProcessId) != threadId || editProcessId != expectedProcessId ||
            GetWindowThreadProcessId(combo, out comboProcessId) != threadId || comboProcessId != expectedProcessId ||
            !IsOwnedWindow(edit, expectedProcessId) || !IsOwnedWindow(combo, expectedProcessId) ||
            !IsChild(dialog, edit) || !IsChild(dialog, combo) || ControlId(edit) != 1001 ||
            !String.Equals(WindowClass(edit), "Edit", StringComparison.OrdinalIgnoreCase) ||
            !String.Equals(WindowClass(combo), "ComboBox", StringComparison.OrdinalIgnoreCase) ||
            !IsWindowVisible(edit) || !IsWindowEnabled(edit) || !IsWindowVisible(combo) || !IsWindowEnabled(combo) ||
            (GetWindowLongW(edit, -16) & 0x0800) != 0)
        {
            LastSaveAsFilenameFocusFailure = "EDIT_COMBO_CONTRACT_FAILED";
            return IntPtr.Zero;
        }
        IntPtr ancestor = combo;
        int ancestorDepth = 0;
        for (; ancestorDepth < 16 && ancestor != IntPtr.Zero; ancestorDepth++)
        {
            if (!IsOwnedWindow(ancestor, expectedProcessId))
            {
                LastSaveAsFilenameFocusFailure = "ANCESTOR_OWNERSHIP_GATE_FAILED: depth=" +
                    ancestorDepth + "; ancestor_hwnd=" + WindowHandleText(ancestor);
                return IntPtr.Zero;
            }
            uint ancestorProcessId;
            uint ancestorThreadId = GetWindowThreadProcessId(ancestor, out ancestorProcessId);
            if (ancestorThreadId != threadId || ancestorProcessId != expectedProcessId)
            {
                LastSaveAsFilenameFocusFailure = "ANCESTOR_THREAD_OR_OWNER_GATE_FAILED: depth=" +
                    ancestorDepth + "; ancestor_hwnd=" + WindowHandleText(ancestor) +
                    "; thread_id=" + ancestorThreadId + "; process_id=" + ancestorProcessId;
                return IntPtr.Zero;
            }
            if (ancestor == dialog) return edit;
            ancestor = GetParent(ancestor);
        }
        LastSaveAsFilenameFocusFailure = "ANCESTOR_DIALOG_NOT_FOUND: depth=" +
            ancestorDepth + "; ancestor_hwnd=" + WindowHandleText(ancestor);
        return IntPtr.Zero;
    }

    private static string WindowHandleText(IntPtr window)
    {
        return "0x" + window.ToInt64().ToString("X");
    }

    public static EditorNativeFocusInfo ObserveOwnedEditorFocus(
        IntPtr expectedEditor, IntPtr main, IntPtr compact, uint expectedProcessId)
    {
        EditorNativeFocusInfo result = new EditorNativeFocusInfo();
        result.expected_hwnd = WindowHandleText(expectedEditor);
        result.window_class = expectedEditor == IntPtr.Zero ? String.Empty : WindowClass(expectedEditor);
        result.active_hwnd = WindowHandleText(IntPtr.Zero);
        result.focus_hwnd = WindowHandleText(IntPtr.Zero);
        result.caret_hwnd = WindowHandleText(IntPtr.Zero);
        bool registeredHandle = expectedEditor == main ||
            (compact != IntPtr.Zero && expectedEditor == compact);
        result.registered_editor_window = registeredHandle &&
            IsOwnedWindow(main, expectedProcessId) &&
            (compact == IntPtr.Zero || IsOwnedWindow(compact, expectedProcessId)) &&
            IsOwnedWindow(expectedEditor, expectedProcessId) &&
            String.Equals(result.window_class, "MDLiteRustEditor", StringComparison.Ordinal);

        IntPtr foreground = GetForegroundWindow();
        uint foregroundProcessId;
        GetWindowThreadProcessId(foreground, out foregroundProcessId);
        result.foreground_is_expected = foreground == expectedEditor &&
            foregroundProcessId == expectedProcessId && IsOwnedWindow(foreground, expectedProcessId);
        if (!result.registered_editor_window)
            return result;

        uint editorProcessId;
        uint threadId = GetWindowThreadProcessId(expectedEditor, out editorProcessId);
        if (threadId == 0 || editorProcessId != expectedProcessId)
            return result;
        GUITHREADINFO info = new GUITHREADINFO();
        info.cbSize = (uint)Marshal.SizeOf(typeof(GUITHREADINFO));
        result.gui_thread_info_available = GetGUIThreadInfo(threadId, ref info);
        if (!result.gui_thread_info_available)
            return result;
        result.active_hwnd = WindowHandleText(info.hwndActive);
        result.focus_hwnd = WindowHandleText(info.hwndFocus);
        result.caret_hwnd = WindowHandleText(info.hwndCaret);
        result.active_is_expected = info.hwndActive == expectedEditor;
        result.focus_is_expected = info.hwndFocus == expectedEditor;
        result.caret_is_expected = info.hwndCaret == expectedEditor;
        result.caret_rect = new int[] {
            info.rcCaret.Left, info.rcCaret.Top, info.rcCaret.Right, info.rcCaret.Bottom
        };
        return result;
    }

    public static bool IsOwnedWindow(IntPtr window, uint expectedProcessId)
    {
        if (window == IntPtr.Zero || !IsWindow(window))
            return false;
        uint actualProcessId;
        GetWindowThreadProcessId(window, out actualProcessId);
        return actualProcessId == expectedProcessId;
    }

    public static int ControlId(IntPtr window)
    {
        return GetDlgCtrlID(window);
    }

    public static int[] WindowRect(IntPtr window)
    {
        RECT rect;
        if (!GetWindowRect(window, out rect))
            throw new InvalidOperationException("GetWindowRect failed for an owned HWND.");
        return new int[] { rect.Left, rect.Top, rect.Right, rect.Bottom };
    }

    public static int[] ListBoxItemRect(
        IntPtr list, int index, uint timeoutMs, uint flags)
    {
        const uint LB_GETITEMRECT = 0x0198;
        RECT rect = new RECT();
        IntPtr memory = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(RECT)));
        try
        {
            Marshal.StructureToPtr(rect, memory, false);
            IntPtr result;
            if (!SendMessageTimeoutW(list, LB_GETITEMRECT, new IntPtr(index), memory,
                    flags, timeoutMs, out result))
                throw new TimeoutException("Listbox item geometry timed out.");
            if (result.ToInt64() == -1)
                throw new InvalidOperationException("Listbox item geometry is unavailable.");
            rect = (RECT)Marshal.PtrToStructure(memory, typeof(RECT));
            return new int[] { rect.Left, rect.Top, rect.Right, rect.Bottom };
        }
        finally
        {
            Marshal.FreeHGlobal(memory);
        }
    }

    public static int FindListBoxItemExact(
        IntPtr list, string value, uint timeoutMs, uint flags)
    {
        const uint LB_FINDSTRINGEXACT = 0x01A2;
        IntPtr text = Marshal.StringToHGlobalUni(value);
        try
        {
            IntPtr result;
            if (!SendMessageTimeoutW(list, LB_FINDSTRINGEXACT, new IntPtr(-1), text,
                    flags, timeoutMs, out result))
                throw new TimeoutException("Listbox text lookup timed out.");
            return result.ToInt32();
        }
        finally
        {
            Marshal.FreeHGlobal(text);
        }
    }

    public static string ListBoxItemText(
        IntPtr list, int index, uint timeoutMs, uint flags)
    {
        const uint LB_GETTEXTLEN = 0x018A;
        const uint LB_GETTEXT = 0x0189;
        IntPtr lengthResult;
        if (!SendMessageTimeoutW(list, LB_GETTEXTLEN, new IntPtr(index), IntPtr.Zero,
                flags, timeoutMs, out lengthResult))
            throw new TimeoutException("Listbox item length timed out.");
        int length = lengthResult.ToInt32();
        if (length < 0 || length > 65536)
            throw new InvalidOperationException("Listbox item length is outside the bounded range.");
        IntPtr buffer = Marshal.AllocHGlobal((length + 1) * 2);
        try
        {
            IntPtr textResult;
            if (!SendMessageTimeoutW(list, LB_GETTEXT, new IntPtr(index), buffer,
                    flags, timeoutMs, out textResult))
                throw new TimeoutException("Listbox item read timed out.");
            return Marshal.PtrToStringUni(buffer, Math.Max(0, textResult.ToInt32()));
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }
    }

    public static int[] ClientRect(IntPtr window)
    {
        RECT rect;
        if (!GetClientRect(window, out rect))
            throw new InvalidOperationException("GetClientRect failed for an owned HWND.");
        return new int[] { rect.Left, rect.Top, rect.Right, rect.Bottom };
    }

    public static long[] OwnedChildren(IntPtr root, uint expectedProcessId)
    {
        if (!IsOwnedWindow(root, expectedProcessId))
            throw new InvalidOperationException("Child enumeration requires an owned root HWND.");
        List<long> result = new List<long>();
        EnumChildWindows(root, delegate(IntPtr window, IntPtr state)
        {
            if (IsOwnedWindow(window, expectedProcessId))
                result.Add(window.ToInt64());
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }

    public static long[] OwnedThreadWindows(IntPtr root, uint expectedProcessId)
    {
        if (!IsOwnedWindow(root, expectedProcessId))
            throw new InvalidOperationException("Thread-window enumeration requires an owned root HWND.");
        uint actualProcessId;
        uint threadId = GetWindowThreadProcessId(root, out actualProcessId);
        if (threadId == 0 || actualProcessId != expectedProcessId)
            throw new InvalidOperationException("The owned HWND thread could not be verified.");
        List<long> result = new List<long>();
        EnumThreadWindows(threadId, delegate(IntPtr window, IntPtr state)
        {
            if (IsOwnedWindow(window, expectedProcessId))
                result.Add(window.ToInt64());
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }

    public static int ClickOwnedAt(
        IntPtr expectedForeground, uint expectedProcessId, int x, int y)
    {
        if (GetForegroundWindow() != expectedForeground ||
            !IsOwnedWindow(expectedForeground, expectedProcessId))
            return -1;
        RECT rect;
        if (!GetWindowRect(expectedForeground, out rect) ||
            x < rect.Left || x >= rect.Right || y < rect.Top || y >= rect.Bottom)
            return -2;
        if (!SetCursorPos(x, y))
            return -3;
        if (GetForegroundWindow() != expectedForeground ||
            !IsOwnedWindow(expectedForeground, expectedProcessId))
            return -1;
        INPUT[] inputs = new INPUT[2];
        inputs[0].type = 0;
        inputs[0].data.mi.dwFlags = 0x0002;
        inputs[1].type = 0;
        inputs[1].data.mi.dwFlags = 0x0004;
        return (int)SendInput(2, inputs, Marshal.SizeOf(typeof(INPUT)));
    }

    public static int ShiftClickOwnedAt(
        IntPtr expectedForeground, uint expectedProcessId, int x, int y)
    {
        if (GetForegroundWindow() != expectedForeground ||
            !IsOwnedWindow(expectedForeground, expectedProcessId))
            return -1;
        RECT rect;
        if (!GetWindowRect(expectedForeground, out rect) ||
            x < rect.Left || x >= rect.Right || y < rect.Top || y >= rect.Bottom)
            return -2;
        if (!SetCursorPos(x, y))
            return -3;
        if (GetForegroundWindow() != expectedForeground ||
            !IsOwnedWindow(expectedForeground, expectedProcessId))
            return -1;

        INPUT shiftDown = KeyInput(0x10, 0, 0);
        if (SendInput(1, new INPUT[] { shiftDown }, Marshal.SizeOf(typeof(INPUT))) != 1)
            return -4;
        int mouseDownSent = 0;
        int mouseUpSent = 0;
        int shiftUpSent = 0;
        try
        {
            if (GetForegroundWindow() != expectedForeground ||
                !IsOwnedWindow(expectedForeground, expectedProcessId))
                return -1;
            INPUT mouseDown = new INPUT();
            mouseDown.type = 0;
            mouseDown.data.mi.dwFlags = 0x0002;
            mouseDownSent = (int)SendInput(
                1, new INPUT[] { mouseDown }, Marshal.SizeOf(typeof(INPUT)));
        }
        finally
        {
            INPUT mouseUp = new INPUT();
            mouseUp.type = 0;
            mouseUp.data.mi.dwFlags = 0x0004;
            mouseUpSent = (int)SendInput(
                1, new INPUT[] { mouseUp }, Marshal.SizeOf(typeof(INPUT)));
            INPUT shiftUp = KeyInput(0x10, 0, KEYEVENTF_KEYUP);
            shiftUpSent = (int)SendInput(
                1, new INPUT[] { shiftUp }, Marshal.SizeOf(typeof(INPUT)));
        }
        if (GetForegroundWindow() != expectedForeground ||
            !IsOwnedWindow(expectedForeground, expectedProcessId))
            return -1;
        if (mouseDownSent != 1 || mouseUpSent != 1 || shiftUpSent != 1)
            return -5;
        return 2;
    }

    public static int[] GetVirtualScreenMetrics()
    {
        return new int[] {
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN)
        };
    }

    public static int NormalizeVirtualCoordinate(int coordinate, int origin, int extent)
    {
        if (extent <= 1) throw new ArgumentOutOfRangeException("extent");
        int maximum = checked(origin + extent - 1);
        if (coordinate < origin || coordinate > maximum)
            throw new ArgumentOutOfRangeException("coordinate");

        double scaled = (double)(coordinate - origin) * 65535.0 / (double)(extent - 1);
        return (int)Math.Round(scaled, MidpointRounding.AwayFromZero);
    }

    public static uint MoveAbsoluteVirtual(
        int x, int y, int virtualX, int virtualY, int virtualWidth, int virtualHeight,
        out int lastError)
    {
        int absoluteX = NormalizeVirtualCoordinate(x, virtualX, virtualWidth);
        int absoluteY = NormalizeVirtualCoordinate(y, virtualY, virtualHeight);
        return SendMouse(absoluteX, absoluteY,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            out lastError);
    }

    public static uint LeftButtonDown(out int lastError)
    {
        return SendMouse(0, 0, MOUSEEVENTF_LEFTDOWN, out lastError);
    }

    public static uint LeftButtonUp(out int lastError)
    {
        return SendMouse(0, 0, MOUSEEVENTF_LEFTUP, out lastError);
    }

    public static bool TryGetCursorPosition(out int x, out int y, out int lastError)
    {
        POINT point;
        bool ok = GetCursorPos(out point);
        lastError = ok ? 0 : Marshal.GetLastWin32Error();
        x = point.X;
        y = point.Y;
        return ok;
    }

    public static bool TryGetThreadCapture(
        uint threadId, out IntPtr hwndCapture, out int lastError)
    {
        GUITHREADINFO info = new GUITHREADINFO();
        info.cbSize = (uint)Marshal.SizeOf(typeof(GUITHREADINFO));
        bool ok = GetGUIThreadInfo(threadId, ref info);
        lastError = ok ? 0 : Marshal.GetLastWin32Error();
        hwndCapture = info.hwndCapture;
        return ok;
    }

    public static bool IsLeftButtonDown()
    {
        return (GetAsyncKeyState(1) & 0x8000) != 0;
    }

    private static uint SendMouse(int dx, int dy, uint flags, out int lastError)
    {
        MOUSEINPUT mouse = new MOUSEINPUT();
        mouse.dx = dx;
        mouse.dy = dy;
        mouse.dwFlags = flags;

        INPUT input = new INPUT();
        input.type = INPUT_MOUSE;
        input.data = new INPUTUNION();
        input.data.mi = mouse;

        uint sent = SendInput(1, new INPUT[] { input }, Marshal.SizeOf(typeof(INPUT)));
        lastError = sent == 1 ? 0 : Marshal.GetLastWin32Error();
        return sent;
    }

    public static bool OwnedClientPointToScreen(
        IntPtr window, uint expectedProcessId, int x, int y, out int screenX, out int screenY)
    {
        screenX = 0;
        screenY = 0;
        if (!IsOwnedWindow(window, expectedProcessId))
            return false;
        POINT point = new POINT { X = x, Y = y };
        if (!ClientToScreen(window, ref point))
            return false;
        screenX = point.X;
        screenY = point.Y;
        return true;
    }

    public static IntPtr OwnedWindowKeyboardLayout(IntPtr window, uint expectedProcessId)
    {
        if (!IsOwnedWindow(window, expectedProcessId))
            return IntPtr.Zero;
        uint actualProcessId;
        uint threadId = GetWindowThreadProcessId(window, out actualProcessId);
        if (threadId == 0 || actualProcessId != expectedProcessId)
            return IntPtr.Zero;
        return GetKeyboardLayout(threadId);
    }

    public static short MapAsciiCharacter(char character, IntPtr keyboardLayout)
    {
        return VkKeyScanExW(character, keyboardLayout);
    }

    public static bool CapsLockIsOn()
    {
        return (GetKeyState(0x14) & 1) != 0;
    }

    private static bool IsOwnedForeground(IntPtr expectedWindow, uint expectedProcessId)
    {
        return GetForegroundWindow() == expectedWindow &&
               IsOwnedWindow(expectedWindow, expectedProcessId);
    }

    private static INPUT KeyInput(ushort virtualKey, ushort scanCode, uint flags)
    {
        INPUT input = new INPUT();
        input.type = INPUT_KEYBOARD;
        input.data.ki.wVk = virtualKey;
        input.data.ki.wScan = scanCode;
        input.data.ki.dwFlags = flags;
        input.data.ki.time = 0;
        input.data.ki.dwExtraInfo = UIntPtr.Zero;
        return input;
    }

    public static int SendVirtualKey(
        IntPtr expectedWindow, uint expectedProcessId,
        ushort virtualKey, ushort[] modifiers)
    {
        if (!IsOwnedForeground(expectedWindow, expectedProcessId))
            return -1;
        List<INPUT> events = new List<INPUT>();
        if (modifiers != null)
        {
            for (int i = 0; i < modifiers.Length; i++)
                events.Add(KeyInput(modifiers[i], 0, 0));
        }
        events.Add(KeyInput(virtualKey, 0, 0));
        events.Add(KeyInput(virtualKey, 0, KEYEVENTF_KEYUP));
        if (modifiers != null)
        {
            for (int i = modifiers.Length - 1; i >= 0; i--)
                events.Add(KeyInput(modifiers[i], 0, KEYEVENTF_KEYUP));
        }
        INPUT[] inputs = events.ToArray();
        return (int)SendInput((uint)inputs.Length, inputs, Marshal.SizeOf(typeof(INPUT)));
    }

    public static int SendUnicodeText(
        IntPtr expectedWindow, uint expectedProcessId, string text)
    {
        if (!IsOwnedForeground(expectedWindow, expectedProcessId))
            return -1;
        List<INPUT> events = new List<INPUT>();
        foreach (char unit in text)
        {
            events.Add(KeyInput(0, unit, KEYEVENTF_UNICODE));
            events.Add(KeyInput(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        INPUT[] inputs = events.ToArray();
        return (int)SendInput((uint)inputs.Length, inputs, Marshal.SizeOf(typeof(INPUT)));
    }
}
'@

$InputSizeBytes = [MdliteRustAcceptanceNative]::AssertInputLayout()
$script:Process = $null
$script:WindowHandle = [IntPtr]::Zero
$script:InputWindowHandle = [IntPtr]::Zero
$script:ProcessId = [uint32]0
$script:DiagnosticPath = $null
$script:PromptOpen = $false
$script:ReportPath = $null
$script:ForegroundLogPath = $null
$script:ImeInitialOpen = $null
$script:ImeProfile = $null
$script:ImeRestorePending = $false
$script:ImeRestoreAttempted = $false
$script:ObservationCallback = $null
$script:Report = [ordered]@{
    schema_version = 1
    status = 'RUNNING'
    configuration = 'Release'
    executable_path = [IO.Path]::GetFullPath($ExecutablePath)
    executable_sha256_before = $null
    executable_sha256_after = $null
    source_tree_sha256_before = $null
    source_tree_sha256_after = $null
    git_head = $null
    harness_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    input_size_bytes = $InputSizeBytes
    pid = $null
    hwnd = $null
    visible = $null
    dpi = $null
    foreground_log_path = $null
    foreground_acquired_utc = $null
    diagnostic_message_timeout_ms = $DiagnosticMessageTimeoutMs
    active_input_profile = $null
    ime_open_status = $null
    input_delivery = 'Visible Release app; ASCII uses current-layout virtual-key SendInput; Japanese packet text is a separate Unicode path; commands and modifiers use virtual keys through the normal window message loop.'
    filename_setup_method = 'NOT_RUN'
    save_as_focus_guard_failure = $null
    save_as_dialog_cleanup = @()
    ascii_keyboard_layout = $null
    ascii_caps_lock_on = $null
    ime_status = 'PENDING'
    ime_open_initial = $null
    ime_open_restored = $null
    physical_typematic_status = 'NOT_RUN: physical keyboard repeat was not exercised.'
    discrete_repeat_status = 'PENDING'
    tabs = $null
    fixtures = @()
    operations = @()
    native_editor_focus_checks = @()
    workspace_settings = $null
    unsaved_close_prompt = $null
    normal_close = $null
    process_still_running = $null
    failure = $null
    failure_kind = $null
    secondary_failures = @()
    result_path = $null
    ime_scripted_reading = $null
    ime_expected_reading = $null
    ime_space_conversion = $null
    ime_candidate_movement = $null
    ime_composition_invariants = @()
    ime_escape_stages = @()
    ime_open_restoration_error = $null
}

function Get-SourceTreeSha256 {
    $files = @()
    foreach ($name in @('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'build.rs')) {
        $path = Join-Path $ProjectRoot $name
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            $files += Get-Item -LiteralPath $path
        }
    }
    $files += @(Get-ChildItem -LiteralPath (Join-Path $ProjectRoot 'src') -Filter '*.rs' -File -Recurse | Sort-Object FullName)
    $entries = foreach ($file in ($files | Sort-Object FullName -Unique)) {
        $relative = $file.FullName.Substring($ProjectRoot.Length).TrimStart('\').Replace('\', '/')
        $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        "$relative $hash"
    }
    $bytes = [Text.Encoding]::UTF8.GetBytes([string]::Join(([char]10).ToString(), [string[]]$entries))
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function ConvertTo-WindowsArgument([string]$Value) {
    if ($Value -notmatch '[\s"]') { return $Value }
    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"')
    $slashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq [char]'\') {
            $slashes++
        } elseif ($character -eq [char]'"') {
            [void]$builder.Append([char]'\', (2 * $slashes + 1))
            [void]$builder.Append([char]'"')
            $slashes = 0
        } else {
            if ($slashes -gt 0) { [void]$builder.Append([char]'\', $slashes) }
            [void]$builder.Append($character)
            $slashes = 0
        }
    }
    if ($slashes -gt 0) { [void]$builder.Append([char]'\', (2 * $slashes)) }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Assert-OwnedWindow([IntPtr]$Window) {
    if (-not [MdliteRustAcceptanceNative]::IsOwnedWindow($Window, $script:ProcessId)) {
        throw 'The target HWND is no longer owned by the launched Release process.'
    }
}

function Set-OwnedForeground(
    [IntPtr]$Window = $(if ($script:InputWindowHandle -ne [IntPtr]::Zero) { $script:InputWindowHandle } else { $script:WindowHandle }),
    [int]$WaitMs = 5000
) {
    Assert-OwnedWindow $Window
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    $activationLogged = $false
    do {
        Assert-OwnedWindow $Window
        $foreground = [MdliteRustAcceptanceNative]::GetForegroundWindow()
        [uint32]$foregroundProcessId = 0
        if ($foreground -ne [IntPtr]::Zero) {
            [void][MdliteRustAcceptanceNative]::GetWindowThreadProcessId(
                $foreground, [ref]$foregroundProcessId)
        }
        if ($foreground -eq $Window -and $foregroundProcessId -eq $script:ProcessId) {
            if ($activationLogged) {
                $confirmed = 'FOREGROUND_CONFIRMED pid={0} hwnd=0x{1:X} foreground_pid={2} foreground_hwnd=0x{3:X}' -f
                    $script:ProcessId, $Window.ToInt64(), $foregroundProcessId, $foreground.ToInt64()
                Write-Host $confirmed
                if (-not [string]::IsNullOrWhiteSpace($script:ForegroundLogPath)) {
                    [IO.File]::AppendAllText(
                        $script:ForegroundLogPath, $confirmed + [Environment]::NewLine,
                        [Text.UTF8Encoding]::new($false))
                }
            }
            if ($null -eq $script:Report.foreground_acquired_utc) {
                $script:Report.foreground_acquired_utc = [DateTime]::UtcNow.ToString('o')
            }
            return
        }
        if (-not $activationLogged) {
            $required = 'FOREGROUND_REQUIRED pid={0} hwnd=0x{1:X}' -f
                $script:ProcessId, $Window.ToInt64()
            Write-Host $required
            if (-not [string]::IsNullOrWhiteSpace($script:ForegroundLogPath)) {
                [IO.File]::AppendAllText(
                    $script:ForegroundLogPath, $required + [Environment]::NewLine,
                    [Text.UTF8Encoding]::new($false))
            }
            $activationLogged = $true
        }
        [void][MdliteRustAcceptanceNative]::SetForegroundWindow($Window)
        if ([DateTime]::UtcNow -ge $deadline) {
            throw "The verified owned HWND did not become foreground within $WaitMs ms."
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "The verified owned HWND did not become foreground within $WaitMs ms."
}

function Send-Key(
    [IntPtr]$TargetWindow,
    [int]$VirtualKey,
    [uint16[]]$Modifiers = @()
) {
    Set-OwnedForeground -Window $TargetWindow
    if ($null -eq $Modifiers) { $Modifiers = [uint16[]]@() }
    $expected = 2 + (2 * $Modifiers.Length)
    $sent = [MdliteRustAcceptanceNative]::SendVirtualKey(
        $TargetWindow, $script:ProcessId, [uint16]$VirtualKey, $Modifiers)
    if ($sent -ne $expected) {
        throw "SendInput delivered $sent of $expected key events to the verified owned foreground window."
    }
}

function Send-UnicodeText(
    [string]$Text,
    [IntPtr]$TargetWindow = [IntPtr]::Zero
) {
    if ($TargetWindow -eq [IntPtr]::Zero) {
        $TargetWindow = if ($script:InputWindowHandle -ne [IntPtr]::Zero) {
            $script:InputWindowHandle
        } else {
            $script:WindowHandle
        }
    }
    Set-OwnedForeground -Window $TargetWindow
    $expected = 2 * $Text.Length
    $sent = [MdliteRustAcceptanceNative]::SendUnicodeText(
        $TargetWindow, $script:ProcessId, $Text)
    if ($sent -ne $expected) {
        throw "SendInput delivered $sent of $expected Unicode key events to the verified owned foreground window."
    }
}

function Send-Ascii(
    [string]$Text,
    [IntPtr]$TargetWindow = [IntPtr]::Zero
) {
    if ([string]::IsNullOrEmpty($Text)) { return }
    if ($TargetWindow -eq [IntPtr]::Zero) {
        $TargetWindow = if ($script:InputWindowHandle -ne [IntPtr]::Zero) {
            $script:InputWindowHandle
        } else {
            $script:WindowHandle
        }
    }
    Set-OwnedForeground -Window $TargetWindow
    $keyboardLayout = [MdliteRustAcceptanceNative]::OwnedWindowKeyboardLayout(
        $TargetWindow, $script:ProcessId)
    if ($keyboardLayout -eq [IntPtr]::Zero) {
        throw 'The exact owned window did not expose its current keyboard layout.'
    }
    $capsLockOn = [MdliteRustAcceptanceNative]::CapsLockIsOn()
    if ($null -eq $script:Report.ascii_caps_lock_on) {
        $script:Report.ascii_caps_lock_on = $capsLockOn
        $script:Report.ascii_keyboard_layout = '0x{0:X}' -f $keyboardLayout.ToInt64()
    } elseif ([bool]$script:Report.ascii_caps_lock_on -ne $capsLockOn -or
              [string]$script:Report.ascii_keyboard_layout -cne ('0x{0:X}' -f $keyboardLayout.ToInt64())) {
        throw 'The observed CapsLock state or owned-window keyboard layout changed during ASCII input.'
    }

    $mappedKeys = @()
    foreach ($character in $Text.ToCharArray()) {
        $code = [int]$character
        if ($code -lt 0x20 -or $code -gt 0x7e) {
            throw 'Send-Ascii accepts printable ASCII characters only.'
        }
        $mapped = [MdliteRustAcceptanceNative]::MapAsciiCharacter($character, $keyboardLayout)
        if ($mapped -eq -1) {
            throw ('The current keyboard layout cannot produce ASCII U+{0:X4}.' -f $code)
        }
        $shiftState = ([int]$mapped -shr 8) -band 0xff
        if (($shiftState -band 0xfe) -ne 0) {
            throw ('ASCII U+{0:X4} requires Ctrl/Alt in the current layout; refusing that mapping.' -f $code)
        }
        $isLetter = ($code -ge 0x41 -and $code -le 0x5a) -or
                    ($code -ge 0x61 -and $code -le 0x7a)
        if ($capsLockOn -and $isLetter) { $shiftState = $shiftState -bxor 1 }
        [uint16[]]$modifiers = @()
        if ($shiftState -band 1) { $modifiers = [uint16[]]@(0x10) }
        $mappedKeys += [pscustomobject]@{
            virtual_key = [int]([int]$mapped -band 0xff)
            modifiers = $modifiers
        }
    }

    foreach ($key in $mappedKeys) {
        $currentLayout = [MdliteRustAcceptanceNative]::OwnedWindowKeyboardLayout(
            $TargetWindow, $script:ProcessId)
        if ($currentLayout -ne $keyboardLayout) {
            throw 'The owned-window keyboard layout changed during ASCII input; no profile switch was requested.'
        }
        if ([MdliteRustAcceptanceNative]::CapsLockIsOn() -ne $capsLockOn) {
            throw 'CapsLock changed during ASCII input; the harness did not modify it.'
        }
        Send-Key $TargetWindow $key.virtual_key $key.modifiers
    }
}

function Assert-SelectionBoundary($State) {
    $bytes = [Text.Encoding]::UTF8.GetBytes([string]$State.source)
    foreach ($offset in @([int]$State.selection.anchor, [int]$State.selection.head)) {
        if ($offset -lt 0 -or $offset -gt $bytes.Length) {
            throw "Diagnostic selection offset $offset is outside the UTF-8 source."
        }
        if ($offset -lt $bytes.Length -and (($bytes[$offset] -band 0xC0) -eq 0x80)) {
            throw "Diagnostic selection offset $offset splits a UTF-8 scalar."
        }
    }
}

function Get-RemainingDiagnosticMessageTimeoutMs([DateTime]$Deadline) {
    $remainingMs = [int][Math]::Floor(($Deadline - [DateTime]::UtcNow).TotalMilliseconds)
    if ($remainingMs -le 0) { return 0 }
    return [int][Math]::Min($DiagnosticMessageTimeoutMs, $remainingMs)
}

function Read-Diagnostic([int]$MessageTimeoutMs = $DiagnosticMessageTimeoutMs) {
    Assert-OwnedWindow $script:WindowHandle
    $previousWriteUtc = $null
    try {
        if (Test-Path -LiteralPath $script:DiagnosticPath -PathType Leaf) {
            $previousWriteUtc = [IO.File]::GetLastWriteTimeUtc($script:DiagnosticPath)
        }
    } catch [IO.IOException] {
        throw [TimeoutException]::new(('Transient diagnostic snapshot stat: ' + $_.Exception.Message), $_.Exception)
    }
    $messageResult = [IntPtr]::Zero
    $sent = [MdliteRustAcceptanceNative]::SendMessageTimeoutW(
        $script:WindowHandle, [uint32]$DiagnosticRefreshMessage,
        [IntPtr]::Zero, [IntPtr]::Zero, [uint32]$SMTO_ABORTIFHUNG,
        [uint32]$MessageTimeoutMs, [ref]$messageResult)
    Assert-OwnedWindow $script:WindowHandle
    if (-not $sent) {
        throw [TimeoutException]::new('SendMessageTimeout did not complete.')
    }
    if ($messageResult.ToInt64() -ne 1) {
        throw [TimeoutException]::new('WM_APP+1 returned LRESULT 0; the owner-thread refresh was deferred.')
    }
    $writeUtc = $null
    try {
        if (-not (Test-Path -LiteralPath $script:DiagnosticPath -PathType Leaf)) {
            throw [TimeoutException]::new('WM_APP+1 returned 1 but no diagnostic snapshot file exists yet.')
        }
        $writeUtc = [IO.File]::GetLastWriteTimeUtc($script:DiagnosticPath)
    } catch [IO.IOException] {
        throw [TimeoutException]::new(('Transient diagnostic snapshot stat: ' + $_.Exception.Message), $_.Exception)
    }
    if ($null -ne $previousWriteUtc -and $writeUtc -le $previousWriteUtc) {
        throw [TimeoutException]::new('WM_APP+1 returned 1 but the diagnostic snapshot timestamp did not advance.')
    }
    try {
        $json = [IO.File]::ReadAllText($script:DiagnosticPath, [Text.Encoding]::UTF8)
    } catch [IO.IOException] {
        throw [TimeoutException]::new(('Transient diagnostic snapshot I/O: ' + $_.Exception.Message), $_.Exception)
    }
    try {
        $state = $json | ConvertFrom-Json -ErrorAction Stop
    } catch {
        throw [TimeoutException]::new(('Diagnostic snapshot JSON is not complete yet: ' + $_.Exception.Message), $_.Exception)
    }
    if ($null -eq $state) {
        throw [TimeoutException]::new('Diagnostic snapshot JSON is incomplete; object is unavailable.')
    }
    $sourceProperty = $state.PSObject.Properties['source']
    $selectionProperty = $state.PSObject.Properties['selection']
    if ($null -eq $sourceProperty -or $null -eq $selectionProperty -or
        $null -eq $sourceProperty.Value -or $null -eq $selectionProperty.Value) {
        throw [TimeoutException]::new('Diagnostic snapshot JSON is incomplete; source or selection is unavailable.')
    }
    $selection = $selectionProperty.Value
    if ($sourceProperty.Value -isnot [string]) {
        throw [TimeoutException]::new('Diagnostic snapshot JSON is incomplete; source is not a string.')
    }
    foreach ($name in @('source', 'selection', 'revision', 'dirty', 'undo_count', 'redo_count', 'raw', 'tables', 'ime', 'caret', 'native_caret')) {
        $property = $state.PSObject.Properties[$name]
        if ($null -eq $property -or $null -eq $property.Value) {
            throw [TimeoutException]::new("Diagnostic snapshot is incomplete; missing '$name'.")
        }
    }
    foreach ($name in @('anchor', 'head')) {
        $property = $selection.PSObject.Properties[$name]
        if ($null -eq $property -or $null -eq $property.Value) {
            throw [TimeoutException]::new("Diagnostic selection is incomplete; missing '$name'.")
        }
        if ($property.Value -isnot [int] -and $property.Value -isnot [long]) {
            throw [TimeoutException]::new("Diagnostic selection is incomplete; '$name' is not an integer.")
        }
    }
    foreach ($name in @('active', 'preedit', 'open_status')) {
        $property = $state.ime.PSObject.Properties[$name]
        if ($null -eq $property -or $null -eq $property.Value) {
            throw [TimeoutException]::new("Diagnostic IME state is incomplete; missing '$name'.")
        }
    }
    Assert-SelectionBoundary $state
    return $state
}

function Wait-Diagnostic(
    [scriptblock]$Condition,
    [string]$Description,
    [int]$WaitMs = $TimeoutMs
) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    $attempts = 0
    $validSnapshotCount = 0
    $lastState = $null
    $lastResponse = 'no diagnostic request completed'
    do {
        $attempts++
        $remainingMs = Get-RemainingDiagnosticMessageTimeoutMs $deadline
        if ($remainingMs -le 0) { break }
        $state = $null
        try {
            $state = Read-Diagnostic $remainingMs
            $lastState = $state
            $validSnapshotCount++
        } catch [TimeoutException] {
            $lastResponse = $_.Exception.Message
        }
        if ($null -ne $state) {
            $lastResponse = 'WM_APP+1 returned 1; fresh valid snapshot revision={0}, selection={1}..{2}, dirty={3}; condition false' -f
                $state.revision, $state.selection.anchor, $state.selection.head, $state.dirty
            if (& $Condition $state) { return $state }
            $state = $null
        }
        if ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 60 }
    } while ([DateTime]::UtcNow -lt $deadline)
    $kind = if ($validSnapshotCount -eq 0) { 'OBSERVER_FAILURE' } else { 'SEMANTIC_WAIT_TIMEOUT' }
    $exception = [TimeoutException]::new($kind + ': timed out after ' + $WaitMs + ' ms waiting for ' + $Description +
        ' (' + $attempts + ' attempts; last observer response: ' + $lastResponse + ').')
    $exception.Data['LastDiagnosticState'] = $lastState
    $exception.Data['ValidSnapshotCount'] = $validSnapshotCount
    throw $exception
}

function Wait-Document(
    [string]$ExpectedSource,
    [int]$ExpectedAnchor,
    [int]$ExpectedHead,
    [bool]$ExpectedDirty
) {
    return Wait-Diagnostic {
        param($state)
        $state.source -ceq $ExpectedSource -and
        [int]$state.selection.anchor -eq $ExpectedAnchor -and
        [int]$state.selection.head -eq $ExpectedHead -and
        [bool]$state.dirty -eq $ExpectedDirty
    } "source/selection/dirty state for '$ExpectedSource'"
}

function Add-Observation([string]$Name, $State) {
    $operation = [pscustomobject]@{
        name = $Name
        revision = [int]$State.revision
        selection = $State.selection
        dirty = [bool]$State.dirty
        undo_count = [int]$State.undo_count
        redo_count = [int]$State.redo_count
        raw = [bool]$State.raw
        table_count = [int]$State.tables
        ime_active = [bool]$State.ime.active
        preedit_length = ([string]$State.ime.preedit).Length
        caret = $State.caret
        native_caret = $State.native_caret
    }
    $script:Report.operations += $operation
    if ($null -ne $script:ObservationCallback) {
        & $script:ObservationCallback $Name $State
    }
}

function Test-SameInputProfile($State) {
    if ($null -eq $script:ImeProfile -or $null -eq $State.active_input_profile) { return $false }
    foreach ($name in @('status', 'profile_type', 'language_id', 'clsid', 'profile_guid', 'hkl')) {
        $expected = $script:ImeProfile.PSObject.Properties[$name]
        $actual = $State.active_input_profile.PSObject.Properties[$name]
        if ($null -eq $expected -or $null -eq $actual -or [string]$expected.Value -cne [string]$actual.Value) {
            return $false
        }
    }
    return $true
}

function Assert-EditorRecoveryImeInvariant($State, $Baseline, [string]$Phase) {
    if (-not (Test-SameInputProfile $State) -or
        $State.source -cne $Baseline.source -or
        [int]$State.selection.anchor -ne [int]$Baseline.selection.anchor -or
        [int]$State.selection.head -ne [int]$Baseline.selection.head -or
        [bool]$State.dirty -ne [bool]$Baseline.dirty -or
        [int]$State.undo_count -ne [int]$Baseline.undo_count -or
        [int]$State.redo_count -ne [int]$Baseline.redo_count) {
        throw ('PRODUCT_ASSERTION: IME ' + $Phase + ' changed the active profile, source, selection, dirty state, or history.')
    }
}

function Get-EditorRecoveryImeCancellationEscapeLimit($Profile) {
    $identity = Get-EditorRecoveryProfileIdentity $Profile
    if ($null -ne $identity -and
        ([string]$identity.clsid).Trim('{}') -ieq '03B5835F-F03C-411B-9CE2-AA23E1171E36' -and
        ([string]$identity.profile_guid).Trim('{}') -ieq 'A76C93D9-5523-4E90-AAFA-4DB112F9AC76') {
        return 4
    }
    return 3
}

function Cancel-EditorRecoveryImeComposition(
    $Baseline, $CurrentState, [string]$Description, [int]$MaxEscapes = 3,
    [switch]$AllowAtokReconversionSelectionCollapse
) {
    $state = $CurrentState
    $stages = @()
    while ($script:ImeEscapesSentForComposition -lt $MaxEscapes) {
        if (-not [bool]$state.ime.active -and [string]::IsNullOrEmpty([string]$state.ime.preedit)) { break }
        $beforePreedit = [string]$state.ime.preedit
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x1B
        $script:ImeEscapesSentForComposition++
        $escapeNumber = $script:ImeEscapesSentForComposition
        $state = Wait-EditorRecoveryStableState -WaitMs 5000 -Description ($Description + ' Escape stage ' + $escapeNumber) -Condition {
            param($value)
            if ($AllowAtokReconversionSelectionCollapse) {
                Assert-EditorRecoveryAtokReconversionCancelInvariant $value $Baseline ($Description + ' Escape stage')
            } else {
                Assert-EditorRecoveryImeInvariant $value $Baseline ($Description + ' Escape stage')
            }
            $true
        }
        $cancelled = -not [bool]$state.ime.active -and [string]::IsNullOrEmpty([string]$state.ime.preedit)
        $afterPreedit = [string]$state.ime.preedit
        $stage = [pscustomobject]@{
            escape_number=$escapeNumber; preedit_before=$beforePreedit; preedit_after=$afterPreedit
            ime_active=[bool]$state.ime.active; composition_cancelled=$cancelled
            transition=if ($cancelled) {'composition_cancelled'} elseif ($afterPreedit -cne $beforePreedit) {'preedit_changed_but_composition_active'} else {'no_diagnostic_preedit_change'}
        }
        $stages += $stage
        $script:Report.ime_escape_stages += $stage
        Add-Observation ($Description + ' cancellation Escape stage ' + $escapeNumber) $state
        if ($cancelled) { break }
    }
    return [pscustomobject]@{
        cancelled=(-not [bool]$state.ime.active -and [string]::IsNullOrEmpty([string]$state.ime.preedit))
        state=$state; stages=$stages
    }
}

function Wait-ImeOpenState([bool]$ExpectedOpen, [string]$ExpectedSource) {
    return Wait-Diagnostic {
        param($state)
        (Test-SameInputProfile $state) -and
        [bool]$state.ime.open_status.available -and
        [bool]$state.ime.open_status.open -eq $ExpectedOpen -and
        $state.source -ceq $ExpectedSource
    } ('app-local IME open state ' + $ExpectedOpen)
}

function Restore-ImeOpenState {
    if (-not $script:ImeRestorePending -or $script:ImeRestoreAttempted) { return }
    $script:ImeRestoreAttempted = $true
    $state = Wait-Diagnostic { param($value) $true } 'fresh diagnostic before IME restoration'
    if (-not (Test-SameInputProfile $state)) {
        throw 'The observed input profile changed during the app-local IME check; no profile-switching key was sent.'
    }
    if ([bool]$state.ime.active -or -not [string]::IsNullOrEmpty([string]$state.ime.preedit)) {
        $baseline = Wait-Diagnostic { param($value) $true } 'IME restoration source and history baseline'
        $maxEscapes = Get-EditorRecoveryImeCancellationEscapeLimit $state.active_input_profile
        $cancelResult = Cancel-EditorRecoveryImeComposition $baseline $state 'cancellation of owned composition' $maxEscapes
        $state = $cancelResult.state
        if (-not $cancelResult.cancelled) {
            throw 'ENVIRONMENT_BLOCKED: owned IME composition remained active after the bounded normal Escape sequence.'
        }
        if ($state.source -cne $baseline.source -or
            [int]$state.selection.anchor -ne [int]$baseline.selection.anchor -or
            [int]$state.selection.head -ne [int]$baseline.selection.head -or
            [bool]$state.dirty -ne [bool]$baseline.dirty -or
            [int]$state.undo_count -ne [int]$baseline.undo_count -or
            [int]$state.redo_count -ne [int]$baseline.redo_count) {
            throw 'PRODUCT_ASSERTION: Escape-based IME cleanup changed source, selection, dirty state, or history.'
        }
    }
    if (-not [bool]$state.ime.open_status.available) {
        throw 'The owned app-local IME open state became unavailable before restoration.'
    }
    $currentOpen = [bool]$state.ime.open_status.open
    if ($currentOpen -ne [bool]$script:ImeInitialOpen) {
        Set-OwnedForeground
        $key = if ($script:ImeInitialOpen) { 0x16 } else { 0x1A }
        Send-Key $script:WindowHandle $key
        $state = Wait-ImeOpenState ([bool]$script:ImeInitialOpen) ([string]$state.source)
    }
    $script:Report.ime_open_restored = [bool]$state.ime.open_status.open
    $script:ImeRestorePending = $false
    if ([bool]$state.ime.open_status.open -ne [bool]$script:ImeInitialOpen) {
        throw 'The owned app-local IME did not return to its original open state.'
    }
}

function Assert-ConfiguredImeSpaceState($State,$Baseline,$Profile,[string]$Phase) {
    if ($State.source -cne $Baseline.source -or
        [int]$State.selection.anchor -ne [int]$Baseline.selection.anchor -or
        [int]$State.selection.head -ne [int]$Baseline.selection.head -or
        [bool]$State.dirty -ne [bool]$Baseline.dirty -or
        [int]$State.undo_count -ne [int]$Baseline.undo_count -or
        [int]$State.redo_count -ne [int]$Baseline.redo_count -or
        [long]$State.document_id -ne [long]$Baseline.document_id -or
        [long]$State.revision -ne [long]$Baseline.revision -or
        [int]$State.tabs -ne [int]$Baseline.tabs -or
        [bool]$State.raw -ne [bool]$Baseline.raw -or
        [string]$State.selection_affinity -cne [string]$Baseline.selection_affinity -or
        -not (Test-EditorRecoveryProfileIdentity $Profile $Baseline.active_input_profile) -or
        [string]$Baseline.active_input_profile.description -cne [string]$Profile.description -or
        -not (Test-EditorRecoveryProfileIdentity $Profile $State.active_input_profile) -or
        [string]$State.active_input_profile.description -cne [string]$Profile.description -or
        $null -eq $State.ime.open_status -or -not [bool]$State.ime.open_status.available -or
        -not [bool]$State.ime.open_status.open -or -not [bool]$State.ime.active -or
        [string]::IsNullOrEmpty([string]$State.ime.preedit) -or
        -not (Test-EditorRecoveryJapaneseText ([string]$State.ime.preedit))) {
        throw ('PRODUCT_ASSERTION: IME Space stage violated source/document/revision/selection/dirty/history/tabs/raw/profile/open/preedit invariants ('+$Phase+').')
    }
}

function Invoke-ConfiguredImeSpaceConversion(
    $Baseline,$Profile,$StartingState,[string]$ExpectedReading,[string]$Description
) {
    $rawReading=[string]$StartingState.ime.preedit
    if ($rawReading -cne $ExpectedReading) {
        throw ('PRODUCT_ASSERTION: '+$Description+' requires the exact full scripted reading before Space.')
    }
    Assert-ConfiguredImeSpaceState $StartingState $Baseline $Profile ($Description+' initial state')

    $exactMicrosoftIme=$false
    if ([string]$Profile.status -ceq 'active' -and
        [string]$Profile.profile_type -ceq 'text_service' -and
        [string]$Profile.language_id -ceq '0x0411') {
        try {
            $profileClsid=[guid]::Parse([string]$Profile.clsid)
            $profileGuid=[guid]::Parse([string]$Profile.profile_guid)
            $exactMicrosoftIme=$profileClsid -eq [guid]'03B5835F-F03C-411B-9CE2-AA23E1171E36' -and
                $profileGuid -eq [guid]'A76C93D9-5523-4E90-AAFA-4DB112F9AC76'
        } catch { $exactMicrosoftIme=$false }
    }
    $maximumSpaces=if($exactMicrosoftIme){3}else{1}
    $profileKind=if($exactMicrosoftIme){'exact-Microsoft-IME'}elseif([string]$Profile.description -ceq 'ATOK'){'ATOK-single-Space'}else{'other-profile-single-Space'}
    $progression=@()
    $state=$StartingState
    for($spaceNumber=1;$spaceNumber -le $maximumSpaces;$spaceNumber++) {
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x20
        try {
            $next=Wait-EditorRecoveryStableState -WaitMs 10000 -Description ($Description+' Space '+$spaceNumber) -Condition {
                param($value)
                Assert-ConfiguredImeSpaceState $value $Baseline $Profile ($Description+' Space '+$spaceNumber)
                return $true
            }
        } catch {
            $_.Exception.Data['ConfiguredImeSpaceProgression']=@($progression)
            $_.Exception.Data['ConfiguredImeSpaceActions']=$spaceNumber
            throw
        }
        $record=[pscustomobject]@{
            phase=$Description
            action='Space'
            space_number=$spaceNumber
            preedit_before=$rawReading
            preedit_after=[string]$next.ime.preedit
            preedit_sha256=(Get-EditorRecoveryBytesSha256 ([Text.Encoding]::UTF8.GetBytes([string]$next.ime.preedit)))
            distinct_from_full_reading=([string]$next.ime.preedit -cne $rawReading)
            document_id=[long]$next.document_id
            revision=[long]$next.revision
            selection=$next.selection
            selection_affinity=[string]$next.selection_affinity
            dirty=[bool]$next.dirty
            undo_count=[int]$next.undo_count
            redo_count=[int]$next.redo_count
            tabs=[int]$next.tabs
            raw=[bool]$next.raw
            profile=$next.active_input_profile
            ime_open=[bool]$next.ime.open_status.open
            ime_active=[bool]$next.ime.active
        }
        $progression+= $record
        $state=$next
        if([string]$state.ime.preedit -cne $rawReading) {
            return [pscustomobject]@{
                state=$state
                distinct=$true
                space_actions=$spaceNumber
                max_spaces=$maximumSpaces
                profile_kind=$profileKind
                progression=@($progression)
            }
        }
    }
    return [pscustomobject]@{
        state=$state
        distinct=$false
        space_actions=$maximumSpaces
        max_spaces=$maximumSpaces
        profile_kind=$profileKind
        progression=@($progression)
    }
}

function Invoke-ConfiguredImeAcceptance([string]$ExpectedSource, [int]$ExpectedHead) {
    $script:Report.ime_open_restoration_error = $null
    $state = Wait-Diagnostic { param($value) $true } 'fresh diagnostic before IME acceptance'
    $profile = $state.active_input_profile
    $open = $state.ime.open_status
    $profileDescription = if ($null -eq $profile) { 'unavailable' } else { [string]$profile.description }
    $profileType = if ($null -eq $profile) { 'unavailable' } else { [string]$profile.profile_type }
    $profileLanguage = if ($null -eq $profile) { 'unavailable' } else { [string]$profile.language_id }
    $profileStatus = if ($null -eq $profile) { 'unavailable' } else { [string]$profile.status }
    if ($null -eq $profile -or [string]$profile.status -cne 'active' -or
        $profileType -cne 'text_service' -or $profileLanguage -cne '0x0411') {
        $script:Report.ime_status = "BLOCKED: the current owner-thread input profile is not an active Japanese text service (description='$profileDescription', language=$profileLanguage, type=$profileType, status=$profileStatus); no input or profile-selection keys were sent."
        return $state
    }
    if ($null -eq $open -or -not [bool]$open.available -or $null -eq $open.open) {
        $reason = if ($null -eq $open) { 'diagnostic field unavailable' } else { [string]$open.error }
        $script:Report.ime_status = "BLOCKED: app-local IME open state is not observable ($reason); no IME keys were sent."
        return $state
    }
    if ([bool]$state.ime.active -or -not [string]::IsNullOrEmpty([string]$state.ime.preedit)) {
        throw 'The owned editor was already composing before the scoped IME acceptance sequence.'
    }

    $script:ImeProfile = $profile
    $script:ImeInitialOpen = [bool]$open.open
    $script:ImeRestoreAttempted = $false
    $script:ImeRestorePending = $true
    $script:Report.ime_open_initial = [bool]$script:ImeInitialOpen
    $script:Report.ime_profile_description = $profileDescription
    $script:Report.ime_profile_type = [string]$profile.profile_type
    $script:Report.ime_profile_language = [string]$profile.language_id

    $reading = 'nihongokyouhaimehenka'
    $expectedReading = 'にほんごきょうはいめへんか'
    $script:Report.ime_scripted_reading = $reading
    $script:Report.ime_expected_reading = $expectedReading
    $script:Report.ime_space_conversion = $null
    $script:Report.ime_candidate_movement = $null
    $script:Report.ime_composition_invariants = @()
    $script:Report.ime_escape_stages = @()
    $primaryFailure = $null
    try {
        if ([bool]$script:ImeInitialOpen) {
            Set-OwnedForeground
            Send-Key $script:WindowHandle 0x1A
            $state = Wait-ImeOpenState $false $ExpectedSource
            Add-Observation 'app-local IME OFF' $state
        }
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x16
        $state = Wait-ImeOpenState $true $ExpectedSource
        Add-Observation 'app-local IME ON' $state
        $baseline = $state

        $script:ImeEscapesSentForComposition = 0
        Set-OwnedForeground
        Send-Ascii $reading $script:WindowHandle
        try {
            $state = Wait-EditorRecoveryStableState -WaitMs 10000 -Description 'complete scripted IME reading' -Condition {
                param($value)
                Assert-EditorRecoveryImeInvariant $value $baseline 'reading composition'
                [bool]$value.ime.active -and [string]$value.ime.preedit -ceq $expectedReading
            }
        } catch {
            if ($_.Exception.Message -match '^OBSERVER_FAILURE:|^PRODUCT_ASSERTION:') { throw }
            $observed = $_.Exception.Data['LastDiagnosticState']
            if ($null -ne $observed) {
                Assert-EditorRecoveryImeInvariant $observed $baseline 'reading composition'
                $script:Report.ime_composition_preedit = [string]$observed.ime.preedit
                $script:Report.ime_status = "BLOCKED: profile '$profileDescription' did not expose the complete raw reading; expected='$expectedReading'; actual='$($observed.ime.preedit)'."
                return $observed
            }
            throw
        }
        $script:Report.ime_composition_preedit = [string]$state.ime.preedit
        if ([string]$state.ime.preedit -cne $expectedReading) {
            $script:Report.ime_status = "BLOCKED: incomplete scripted reading; expected='$expectedReading'; actual='$($state.ime.preedit)'."
            return $state
        }
        $script:Report.ime_composition_invariants += 'reading: source, selection, dirty state, history, and profile unchanged'
        Add-Observation 'configured IME complete scripted reading' $state

        $rawPreedit = [string]$state.ime.preedit
        $spaceConversion = $null
        try {
            $spaceConversion = Invoke-ConfiguredImeSpaceConversion $baseline $profile $state $expectedReading 'scripted reading conversion'
        } catch {
            $lastConversionState = $_.Exception.Data['LastDiagnosticState']
            $script:Report.ime_space_conversion = @{
                status='NOT_RUN'; before_preedit=$rawPreedit
                after_preedit=if($null -ne $lastConversionState){[string]$lastConversionState.ime.preedit}else{$null}
                space_actions=$_.Exception.Data['ConfiguredImeSpaceActions']
                progression=@($_.Exception.Data['ConfiguredImeSpaceProgression'])
                reason=[string]$_.Exception.Message
            }
            if ($_.Exception.Message -match '^OBSERVER_FAILURE:|^PRODUCT_ASSERTION:') { throw }
            if ($null -eq $lastConversionState) { throw }
            $script:Report.ime_status = "BLOCKED: Space conversion for '$reading' was not observable as a stable, changed Japanese preedit."
            return $lastConversionState
        }
        if (-not [bool]$spaceConversion.distinct) {
            $script:Report.ime_space_conversion = @{
                status='NOT_RUN'; before_preedit=$rawPreedit; after_preedit=[string]$spaceConversion.state.ime.preedit
                space_actions=[int]$spaceConversion.space_actions; max_spaces=[int]$spaceConversion.max_spaces
                profile_kind=[string]$spaceConversion.profile_kind; progression=@($spaceConversion.progression)
                reason='No distinct stable Japanese preedit was observed; Enter was withheld.'
            }
            $script:Report.ime_status = "BLOCKED: Space conversion for '$reading' did not expose a distinct stable Japanese preedit after $($spaceConversion.space_actions) bounded Space action(s)."
            return $spaceConversion.state
        }
        $converted = $spaceConversion.state
        $convertedPreedit = [string]$converted.ime.preedit
        $script:Report.ime_space_conversion = @{
            status='PASS'; before_preedit=$rawPreedit; after_preedit=$convertedPreedit
            stable_for_ms=250; space_actions=[int]$spaceConversion.space_actions
            max_spaces=[int]$spaceConversion.max_spaces; profile_kind=[string]$spaceConversion.profile_kind
            progression=@($spaceConversion.progression)
        }
        $script:Report.ime_conversion_candidate = $convertedPreedit
        $script:Report.ime_composition_invariants += ('Space conversion ('+[int]$spaceConversion.space_actions+' action(s)): source, document, revision, selection, dirty state, history, tabs, raw mode, and profile unchanged')
        Add-Observation ('configured IME stable Space conversion after '+[int]$spaceConversion.space_actions+' action(s)') $converted

        $candidateMovementObserved = $false
        $candidateKeys = @()
        $opened = $null
        $moved = $null
        try {
            Set-OwnedForeground
            Send-Key $script:WindowHandle 0x20
            $candidateKeys += 'Space'
            $opened = Wait-EditorRecoveryStableState -WaitMs 5000 -Description 'stable candidate opening or cycling after conversion' -Condition {
                param($value)
                Assert-ConfiguredImeSpaceState $value $baseline $profile 'candidate opening'
                return $true
            }
            Add-Observation 'configured IME candidate opening Space' $opened
            $moved = $opened
            if ([string]$opened.ime.preedit -ceq $convertedPreedit) {
                # Candidate-list cycling remains a separate, bounded stage.
                Set-OwnedForeground
                Send-Key $script:WindowHandle 0x20
                $candidateKeys += 'Space'
                $moved = Wait-EditorRecoveryStableState -WaitMs 5000 -Description 'distinct stable candidate cycling after one additional Space' -Condition {
                    param($value)
                    Assert-ConfiguredImeSpaceState $value $baseline $profile 'candidate cycling'
                    [string]$value.ime.preedit -cne $convertedPreedit
                }
            }
            $candidateMovementObserved = $true
            $state = $moved
            $script:Report.ime_candidate_movement = @{
                status='PASS'; key='Space'; key_sequence=@($candidateKeys); before_preedit=$convertedPreedit
                conversion_space_actions=[int]$spaceConversion.space_actions
                candidate_opening_space_actions=1
                candidate_cycling_space_actions=[Math]::Max(0,$candidateKeys.Count-1)
                candidate_space_actions=[int]$candidateKeys.Count
                total_space_actions=[int]$spaceConversion.space_actions+[int]$candidateKeys.Count
                candidate_opening_preedit=[string]$opened.ime.preedit
                after_preedit=[string]$moved.ime.preedit; stable_for_ms=250
                evidence='distinct stable owner-thread preedit after bounded Space cycling'
                candidate_popup_visual_review='NOT_RUN'
            }
            $script:Report.ime_composition_invariants += 'Candidate Space opening/cycling: source, document, revision, selection, dirty state, history, tabs, raw mode, and profile unchanged'
            Add-Observation 'configured IME candidate movement with distinct readback' $moved
        } catch {
            if ($_.Exception.Message -match '^OBSERVER_FAILURE:|^PRODUCT_ASSERTION:') { throw }
            $lastCandidateState = $_.Exception.Data['LastDiagnosticState']
            if ($null -eq $lastCandidateState) { throw }
            Assert-ConfiguredImeSpaceState $lastCandidateState $baseline $profile 'candidate movement recovery'
            $script:Report.ime_candidate_movement = @{
                status='NOT_RUN'; key='Space'; key_sequence=@($candidateKeys); before_preedit=$convertedPreedit
                conversion_space_actions=[int]$spaceConversion.space_actions
                candidate_opening_space_actions=[Math]::Min(1,$candidateKeys.Count)
                candidate_cycling_space_actions=[Math]::Max(0,$candidateKeys.Count-1)
                candidate_space_actions=[int]$candidateKeys.Count
                total_space_actions=[int]$spaceConversion.space_actions+[int]$candidateKeys.Count
                candidate_opening_preedit=if($null -ne $opened){[string]$opened.ime.preedit}else{$null}
                after_preedit=[string]$lastCandidateState.ime.preedit
                reason='No distinct stable preedit was observed; candidate-list movement is not exposed by the diagnostic contract.'
                candidate_popup_visual_review='NOT_RUN'
            }
            $script:Report.ime_composition_invariants += 'Candidate Space attempt: source, document, revision, selection, dirty state, history, tabs, raw mode, and profile unchanged'
            $state = $lastCandidateState
        }

        $maxEscapes = Get-EditorRecoveryImeCancellationEscapeLimit $profile
        $cancelResult = Cancel-EditorRecoveryImeComposition $baseline $state 'configured IME reading' $maxEscapes
        $state = $cancelResult.state
        if (-not $cancelResult.cancelled) {
            $script:Report.ime_status = 'BLOCKED: native composition remained active after the bounded ordinary Escape sequence.'
            return $state
        }
        $script:Report.ime_composition_invariants += 'Escape: source, selection, dirty state, history, and profile unchanged'

        $script:ImeEscapesSentForComposition = 0
        Set-OwnedForeground
        Send-Ascii $reading $script:WindowHandle
        try {
            $state = Wait-EditorRecoveryStableState -WaitMs 10000 -Description 'second complete scripted IME reading' -Condition {
                param($value)
                Assert-EditorRecoveryImeInvariant $value $baseline 'commit reading composition'
                [bool]$value.ime.active -and [string]$value.ime.preedit -ceq $expectedReading
            }
        } catch {
            if ($_.Exception.Message -match '^OBSERVER_FAILURE:|^PRODUCT_ASSERTION:') { throw }
            $observed = $_.Exception.Data['LastDiagnosticState']
            if ($null -ne $observed) {
                Assert-EditorRecoveryImeInvariant $observed $baseline 'commit reading composition'
                $script:Report.ime_composition_preedit = [string]$observed.ime.preedit
                $script:Report.ime_status = "BLOCKED: second reading was incomplete; expected='$expectedReading'; actual='$($observed.ime.preedit)'."
                return $observed
            }
            throw
        }
        $script:Report.ime_composition_preedit = [string]$state.ime.preedit
        if ([string]$state.ime.preedit -cne $expectedReading) {
            $script:Report.ime_status = "BLOCKED: second reading was incomplete; expected='$expectedReading'; actual='$($state.ime.preedit)'."
            return $state
        }
        $script:Report.ime_composition_invariants += 'commit reading: source, selection, dirty state, history, and profile unchanged'
        $rawCommitPreedit = [string]$state.ime.preedit
        $commitSpaceConversion = $null
        try {
            $commitSpaceConversion = Invoke-ConfiguredImeSpaceConversion $baseline $profile $state $expectedReading 'pre-Enter conversion'
        } catch {
            $script:Report.ime_space_conversion.commit_status='NOT_RUN'
            $script:Report.ime_space_conversion.commit_before_preedit=$rawCommitPreedit
            $script:Report.ime_space_conversion.commit_after_preedit=if($null -ne $_.Exception.Data['LastDiagnosticState']){[string]$_.Exception.Data['LastDiagnosticState'].ime.preedit}else{$null}
            $script:Report.ime_space_conversion.commit_space_actions=$_.Exception.Data['ConfiguredImeSpaceActions']
            $script:Report.ime_space_conversion.commit_progression=@($_.Exception.Data['ConfiguredImeSpaceProgression'])
            $script:Report.ime_space_conversion.commit_failure=[string]$_.Exception.Message
            throw
        }
        if (-not [bool]$commitSpaceConversion.distinct) {
            $script:Report.ime_space_conversion.commit_status='NOT_RUN'
            $script:Report.ime_space_conversion.commit_before_preedit=$rawCommitPreedit
            $script:Report.ime_space_conversion.commit_after_preedit=[string]$commitSpaceConversion.state.ime.preedit
            $script:Report.ime_space_conversion.commit_space_actions=[int]$commitSpaceConversion.space_actions
            $script:Report.ime_space_conversion.commit_max_spaces=[int]$commitSpaceConversion.max_spaces
            $script:Report.ime_space_conversion.commit_progression=@($commitSpaceConversion.progression)
            throw 'ENVIRONMENT_BLOCKED: no distinct stable Japanese preedit was observed before Enter; Enter was not sent.'
        }
        $converted = $commitSpaceConversion.state
        $script:Report.ime_conversion_candidate = [string]$converted.ime.preedit
        $script:Report.ime_space_conversion.commit_preedit = [string]$converted.ime.preedit
        $script:Report.ime_space_conversion.commit_stable_for_ms = 250
        $script:Report.ime_space_conversion.commit_status = 'PASS_DISTINCT_JAPANESE_PREEDIT_BEFORE_ENTER'
        $script:Report.ime_space_conversion.commit_space_actions = [int]$commitSpaceConversion.space_actions
        $script:Report.ime_space_conversion.commit_max_spaces = [int]$commitSpaceConversion.max_spaces
        $script:Report.ime_space_conversion.commit_profile_kind = [string]$commitSpaceConversion.profile_kind
        $script:Report.ime_space_conversion.commit_progression = @($commitSpaceConversion.progression)
        $script:Report.ime_composition_invariants += ('pre-Enter Space conversion ('+[int]$commitSpaceConversion.space_actions+' action(s)): source, document, revision, selection, dirty state, history, tabs, raw mode, and profile unchanged')
        Add-Observation ('configured IME stable conversion before commit after '+[int]$commitSpaceConversion.space_actions+' action(s)') $converted

        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x0D
        $baseUndoCount = [int]$baseline.undo_count
        $baseRedoCount = [int]$baseline.redo_count
        $state = Wait-EditorRecoveryStableState -WaitMs 10000 -Description 'Enter commit of the configured IME conversion' -Condition {
            param($value)
            (Test-SameInputProfile $value) -and
                -not [bool]$value.ime.active -and
                [string]::IsNullOrEmpty([string]$value.ime.preedit) -and
                $value.source.StartsWith($ExpectedSource, [StringComparison]::Ordinal) -and
                $value.source.Length -gt $ExpectedSource.Length -and
                -not $value.source.Substring($ExpectedSource.Length).Contains([char]10) -and
                (Test-EditorRecoveryJapaneseText $value.source.Substring($ExpectedSource.Length)) -and
                [int]$value.undo_count -eq ($baseUndoCount + 1) -and
                [int]$value.redo_count -eq 0 -and
                [bool]$value.dirty -eq $true -and
                [int]$value.selection.anchor -eq [Text.Encoding]::UTF8.GetByteCount([string]$value.source) -and
                [int]$value.selection.head -eq [Text.Encoding]::UTF8.GetByteCount([string]$value.source)
        }
        $committedSource = [string]$state.source
        $committedEnd = [Text.Encoding]::UTF8.GetByteCount($committedSource)
        $script:Report.ime_committed_suffix = $committedSource.Substring($ExpectedSource.Length)
        if ($script:Report.ime_committed_suffix -cne [string]$converted.ime.preedit) {
            throw 'PRODUCT_ASSERTION: Enter committed a suffix different from the stable converted candidate.'
        }
        Add-Observation 'configured IME Enter commit' $state

        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
        $state = Wait-Diagnostic {
            param($value)
            $value.source -ceq $ExpectedSource -and
                [int]$value.selection.anchor -eq [int]$baseline.selection.anchor -and
                [int]$value.selection.head -eq [int]$baseline.selection.head -and
                [bool]$value.dirty -eq [bool]$baseline.dirty -and
                [int]$value.undo_count -eq $baseUndoCount -and
                [int]$value.redo_count -eq 1
        } 'Undo configured IME commit'
        Add-Observation 'Undo configured IME commit' $state
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x59 ([uint16[]]@(0x11))
        $state = Wait-Diagnostic {
            param($value)
            $value.source -ceq $committedSource -and
                [int]$value.selection.anchor -eq $committedEnd -and
                [int]$value.selection.head -eq $committedEnd -and
                [bool]$value.dirty -eq $true -and
                [int]$value.undo_count -eq ($baseUndoCount + 1) -and
                [int]$value.redo_count -eq 0
        } 'Redo configured IME commit'
        Add-Observation 'Redo configured IME commit' $state
        $movementLabel = if ($candidateMovementObserved) { 'candidate movement read back' } else { 'candidate movement NOT_RUN' }
        $script:Report.ime_status = if ($candidateMovementObserved) {
            "PASS: active configured profile '$profileDescription' (language $($profile.language_id)); full scripted reading, stable Space conversion after $($spaceConversion.space_actions) action(s), $($candidateKeys.Count) candidate-stage Space action(s), Escape cancellation, Enter commit, and one-unit Undo/Redo were observed; candidate UI review remains NOT_RUN."
        } else {
            "PARTIAL: active configured profile '$profileDescription' (language $($profile.language_id)); full reading, stable conversion after $($spaceConversion.space_actions) action(s), Escape, commit, and Undo/Redo passed; $movementLabel."
        }
    } catch {
        $primaryFailure = $_.Exception
    } finally {
        try {
            Restore-ImeOpenState
        } catch {
            $script:Report.ime_open_restoration_error = $_.Exception.Message
            if ($null -eq $primaryFailure) { throw }
        }
    }
    if ($null -ne $primaryFailure) { throw $primaryFailure }
    return $state
}

function Switch-ToSource([string]$ExpectedSource) {
    $state = Wait-Diagnostic { param($value) $true } 'fresh diagnostic while selecting a fixture tab'
    $expectedTabs = [int]$state.tabs
    if ($expectedTabs -lt 1 -or $expectedTabs -gt 3) {
        throw 'The owned EditorRecovery fixture process must have one to three expected tabs.'
    }
    for ($attempt = 0; $attempt -lt $expectedTabs; $attempt++) {
        if ($state.source -ceq $ExpectedSource) { return $state }
        if ([int]$state.tabs -ne $expectedTabs) { throw 'The owned fixture tab count changed during selection.' }
        $oldDocumentId = [long]$state.document_id
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x09 ([uint16[]]@(0x11))
        $state = Wait-Diagnostic {
            param($value)
            [int]$value.tabs -eq $expectedTabs -and [long]$value.document_id -ne $oldDocumentId
        } 'Ctrl+Tab switch to the requested synthetic fixture'
    }
    if ($state.source -ceq $ExpectedSource) { return $state }
    throw 'The requested synthetic fixture was not active after one bounded tab cycle.'
}

function Wait-OwnedConfirmation([int]$WaitMs = 2500) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    do {
        $window = [MdliteRustAcceptanceNative]::GetForegroundWindow()
        if ($window -ne [IntPtr]::Zero -and
            [MdliteRustAcceptanceNative]::IsOwnedWindow($window, $script:ProcessId) -and
            [MdliteRustAcceptanceNative]::WindowClass($window) -eq '#32770' -and
            [MdliteRustAcceptanceNative]::WindowTitle($window) -eq 'MDLite') {
            return $window
        }
        Start-Sleep -Milliseconds 50
        $script:Process.Refresh()
        if ($script:Process.HasExited) { return [IntPtr]::Zero }
    } while ([DateTime]::UtcNow -lt $deadline)
    return [IntPtr]::Zero
}

function Save-ActiveFixture([string]$ExpectedSource, [string]$Path) {
    $expectedEnd = [Text.Encoding]::UTF8.GetByteCount($ExpectedSource)
    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x53 ([uint16[]]@(0x11))
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $ExpectedSource -and
        [int]$value.selection.anchor -eq $expectedEnd -and
        [int]$value.selection.head -eq $expectedEnd -and
        -not [bool]$value.dirty
    } 'save of the active synthetic fixture'
    if ([IO.File]::ReadAllText($Path, [Text.Encoding]::UTF8) -cne $ExpectedSource) {
        throw "The owned fixture did not save with the exact expected source: $Path"
    }
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
        throw "The UTF-8 no-BOM synthetic fixture acquired an unexpected BOM: $Path"
    }
    Add-Observation ('save synthetic fixture ' + [IO.Path]::GetFileName($Path)) $state
}

function Save-UnsavedEditorRecoveryFixture([string]$ExpectedSource, [string]$Path) {
    if (Test-Path -LiteralPath $Path) {
        throw 'FIXTURE_PATH_ALREADY_EXISTS: refusing to overwrite an owned Save As target.'
    }
    $expectedEnd = [Text.Encoding]::UTF8.GetByteCount($ExpectedSource)
    Set-OwnedForeground -Window $script:WindowHandle
    Send-Key $script:WindowHandle 0x53 ([uint16[]]@(0x11))
    $dialog = Wait-UiThreadWindow '#32770' '名前を付けて保存' 30000
    Assert-OwnedWindow $dialog
    $saveAsEvidence = $null
    try {
        try {
            $field = Get-UiControlById -Root $dialog -ControlId 1001 -Class 'EDIT' -RequireVisible -RequireEnabled
        } catch {
            throw ('SETUP_FAILURE: Save As filename Edit1001 is unavailable: ' + $_.Exception.Message)
        }
        try {
            if (-not [MdliteRustAcceptanceNative]::IsOwnedSaveAsDialog($dialog, $script:WindowHandle, $script:ProcessId)) {
                throw 'SETUP_FAILURE: Save As did not match the owned standard caption/class/Main-owner contract.'
            }
            # File name (N) was observed and Alt+N verified on this common dialog.
            # Recursive UIA host resolution exceeded a bounded 20s probe; native
            # mnemonic routing plus exact focus/caret/ancestor checks avoids that provider call.
            Send-Key $dialog 0x4E ([uint16[]]@(0x12))
            $focusDeadline = [DateTime]::UtcNow.AddSeconds(2)
            $editHasFocus = $false
            do {
                $editHasFocus = [MdliteRustAcceptanceNative]::GetSaveAsFilenameFocus(
                    $dialog, $script:WindowHandle, $script:ProcessId) -eq $field.Handle
                if ($editHasFocus) { break }
                Start-Sleep -Milliseconds 25
            } while ([DateTime]::UtcNow -lt $focusDeadline)
            if (-not $editHasFocus) { throw 'SETUP_FAILURE: Save As Alt+N did not focus the unique writable owned filename Edit1001 with its caret.' }
            Assert-SaveAsFilenameFocus $dialog $field.Handle
            $script:Report.filename_setup_method = 'Verified owned SaveAs filename Edit only: bounded WM_SETTEXT, then exact WM_GETTEXT; not editor/IME keyboard proof.'
            [MdliteRustAcceptanceNative]::SetOwnedSaveAsFilename(
                $dialog, $script:WindowHandle, $field.Handle, $script:ProcessId, $Path)
            Assert-SaveAsFilenameFocus $dialog $field.Handle
            $readbackDeadline = [DateTime]::UtcNow.AddSeconds(2)
            $filenameReadback = ''
            do {
                Assert-SaveAsFilenameFocus $dialog $field.Handle
                $filenameReadback = [MdliteRustAcceptanceNative]::OwnedControlText($field.Handle, $script:ProcessId)
                if ([string]::Equals($filenameReadback, $Path, [StringComparison]::Ordinal)) { break }
                Start-Sleep -Milliseconds 25
            } while ([DateTime]::UtcNow -lt $readbackDeadline)
            if (-not [string]::Equals($filenameReadback, $Path, [StringComparison]::Ordinal)) {
                throw ('SETUP_FAILURE: Save As filename readback differed from the typed target; expected=' + $Path + '; actual=' + $filenameReadback)
            }
            Assert-SaveAsFilenameFocus $dialog $field.Handle
            $editHasFocusAfterType = $true
            $hostHandle = [MdliteRustAcceptanceNative]::GetParent($field.Handle)
            [uint32]$fieldProcessId = 0
            $fieldThreadId = [MdliteRustAcceptanceNative]::GetWindowThreadProcessId($field.Handle, [ref]$fieldProcessId)
            $saveAsEvidence = [pscustomobject]@{
                case_id = [string]$script:CurrentEditorRecoveryCaseId
                dialog_hwnd = '0x{0:X}' -f $dialog.ToInt64()
                dialog_owned_by_editor = $true
                dialog_main_owner_hwnd = '0x{0:X}' -f $script:WindowHandle.ToInt64()
                filename_identity_method = 'owned SaveAs File name (N) mnemonic + native focus/caret/Edit/ComboBox/ancestor checks'
                filename_uia_identity_status = 'NOT_QUERIED'
                filename_host_class = [MdliteRustAcceptanceNative]::WindowClass($hostHandle)
                filename_host_hwnd = '0x{0:X}' -f $hostHandle.ToInt64()
                filename_host_owned_by_editor = $true
                filename_host_native_descendant_of_dialog = $true
                filename_edit_hwnd = '0x{0:X}' -f $field.Handle.ToInt64()
                filename_edit_control_id = 1001
                filename_edit_class = [MdliteRustAcceptanceNative]::WindowClass($field.Handle)
                filename_edit_native_child_of_host = $true
                filename_edit_owned_by_editor = $true
                filename_edit_process_id = $fieldProcessId
                filename_edit_thread_id = $fieldThreadId
                filename_edit_writable = $true
                filename_edit_has_native_caret = $true
                filename_edit_has_keyboard_focus_before_type = $editHasFocus
                filename_edit_has_keyboard_focus_after_type = $editHasFocusAfterType
                typed_filename = $Path
                filename_entry_method = 'bounded owned WM_SETTEXT fixture setup; not editor keyboard input'
                filename_readback = $filenameReadback
                filename_readback_method = 'bounded owned WM_GETTEXT'
                filename_readback_timeout_ms = 1000
                readback_matches_typed_filename = $true
                saved_bytes_sha256 = $null
                status = 'FILENAME_READBACK_PASS'
            }
            $script:Report.save_as_dialog_evidence += $saveAsEvidence
        } catch {
            if ($_.Exception.Message -match '^SETUP_FAILURE:') { throw }
            throw ('SETUP_FAILURE: Save As native filename identity, focus, or text readback could not be verified: ' + $_.Exception.Message)
        }
        Assert-SaveAsFilenameFocus $dialog $field.Handle
        Send-Key $dialog 0x0D
        Wait-UiWindowGone $dialog 30000
    } finally {
        $cancel = [ordered]@{
            dialog_hwnd = '0x{0:X}' -f $dialog.ToInt64()
            status = 'NOT_NEEDED'
            dialog_still_present = $false
            reason = $null
        }
        if ([MdliteRustAcceptanceNative]::IsWindow($dialog)) {
            try {
                if (-not [MdliteRustAcceptanceNative]::IsOwnedSaveAsDialog($dialog, $script:WindowHandle, $script:ProcessId)) {
                    throw 'Owned SaveAs cancellation identity changed; no Escape was sent.'
                }
                Send-Key $dialog 0x1B
                # Escape insertion is asynchronous too. Confirm modal teardown
                # before the outer failure-finally asks the borrowed App for diagnostics.
                Wait-UiWindowGone $dialog 30000
                $cancel.status = 'PASS'
            } catch {
                # Retain the original SaveAs error; this is separate cleanup evidence.
                $cancel.status = 'BLOCKED'
                $cancel.reason = $_.Exception.Message
                $cancel.dialog_still_present = [MdliteRustAcceptanceNative]::IsWindow($dialog)
            }
        }
        $script:Report.save_as_dialog_cleanup += [pscustomobject]$cancel
        $script:InputWindowHandle = $script:WindowHandle
    }
    $state = Wait-Document $ExpectedSource $expectedEnd $expectedEnd $false
    $expectedBytes = [Text.UTF8Encoding]::new($false).GetBytes($ExpectedSource)
    if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($Path))) -cne
        (Get-EditorRecoveryBytesSha256 $expectedBytes)) {
        throw 'PRODUCT_ASSERTION: first save of the owned untitled fixture wrote unexpected bytes.'
    }
    $saveAsEvidence.status = 'PASS'
    $saveAsEvidence.saved_bytes_sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    Add-Observation 'save the clean untitled fixture to its owned synthetic path' $state
}

function Assert-SaveAsFilenameFocus([IntPtr]$Dialog, [IntPtr]$Field) {
    $observedField = [MdliteRustAcceptanceNative]::GetSaveAsFilenameFocus(
        $Dialog, $script:WindowHandle, $script:ProcessId)
    if ($observedField -ne $Field) {
        $failureReason = [string][MdliteRustAcceptanceNative]::LastSaveAsFilenameFocusFailure
        if ($observedField -ne [IntPtr]::Zero -and [string]::IsNullOrEmpty($failureReason)) {
            $failureReason = 'VALID_FILENAME_HANDLE_MISMATCH'
        }
        $script:Report.save_as_focus_guard_failure = [ordered]@{
            reason = $failureReason
            dialog_hwnd_numeric = $Dialog.ToInt64()
            field_hwnd_numeric = $Field.ToInt64()
            returned_hwnd_numeric = $observedField.ToInt64()
        }
        throw 'SETUP_FAILURE: Save As filename focus/caret, writable control, or owned ancestor identity changed.'
    }
}

function Assert-UiOwnedWindow([IntPtr]$Window) {
    if (-not [MdliteRustAcceptanceNative]::IsOwnedWindow($Window, $script:ProcessId)) {
        throw [System.InvalidOperationException]::new('OWNERSHIP_GATE')
    }
}

function Get-UiOwnedChildren([IntPtr]$Root) {
    Assert-UiOwnedWindow $Root
    foreach ($handleValue in [MdliteRustAcceptanceNative]::OwnedChildren($Root, $script:ProcessId)) {
        $handle = [IntPtr]([long]$handleValue)
        if (-not [MdliteRustAcceptanceNative]::IsOwnedWindow($handle, $script:ProcessId)) {
            throw [System.InvalidOperationException]::new('OWNERSHIP_GATE')
        }
        [pscustomobject]@{
            Handle = $handle
            Id = [MdliteRustAcceptanceNative]::ControlId($handle)
            Class = [MdliteRustAcceptanceNative]::WindowClass($handle)
            Text = [MdliteRustAcceptanceNative]::WindowTitle($handle)
            Visible = [MdliteRustAcceptanceNative]::IsWindowVisible($handle)
            Enabled = [MdliteRustAcceptanceNative]::IsWindowEnabled($handle)
            Rect = [MdliteRustAcceptanceNative]::WindowRect($handle)
        }
    }
}

function Get-UiControlById(
    [IntPtr]$Root,
    [int]$ControlId,
    [string]$Class = '',
    [switch]$RequireVisible,
    [switch]$RequireEnabled
) {
    $matches = @(
        Get-UiOwnedChildren $Root | Where-Object {
            $_.Id -eq $ControlId -and
            ([string]::IsNullOrEmpty($Class) -or $_.Class -ieq $Class) -and
            (-not $RequireVisible -or $_.Visible) -and
            (-not $RequireEnabled -or $_.Enabled)
        }
    )
    if ($matches.Count -ne 1) {
        throw [System.NotSupportedException]::new('CONTROL_UNAVAILABLE')
    }
    return $matches[0]
}

function Invoke-UiOwnedClick([IntPtr]$ForegroundWindow, [int]$ScreenX, [int]$ScreenY) {
    Assert-UiOwnedWindow $ForegroundWindow
    Set-OwnedForeground -Window $ForegroundWindow
    $sent = [MdliteRustAcceptanceNative]::ClickOwnedAt(
        $ForegroundWindow, $script:ProcessId, $ScreenX, $ScreenY)
    if ($sent -ne 2) {
        throw [System.InvalidOperationException]::new('OWNED_CLICK_FAILED')
    }
}

function Invoke-UiControlClick([IntPtr]$ForegroundWindow, $Control) {
    Assert-UiOwnedWindow $Control.Handle
    $bounds = [int[]]$Control.Rect
    $x = [int][Math]::Floor(($bounds[0] + $bounds[2]) / 2)
    $y = [int][Math]::Floor(($bounds[1] + $bounds[3]) / 2)
    Invoke-UiOwnedClick $ForegroundWindow $x $y
}

function Invoke-UiControlById(
    [IntPtr]$ForegroundWindow,
    [int]$ControlId,
    [string]$Class = 'BUTTON'
) {
    $control = Get-UiControlById -Root $ForegroundWindow -ControlId $ControlId `
        -Class $Class -RequireVisible -RequireEnabled
    Invoke-UiControlClick $ForegroundWindow $control
}

function Send-UiOwnedMessage(
    [IntPtr]$Window,
    [uint32]$Message,
    [IntPtr]$WParam = [IntPtr]::Zero,
    [IntPtr]$LParam = [IntPtr]::Zero,
    [int]$MessageTimeoutMs = 1000
) {
    Assert-UiOwnedWindow $Window
    $result = [IntPtr]::Zero
    $sent = [MdliteRustAcceptanceNative]::SendMessageTimeoutW(
        $Window, $Message, $WParam, $LParam, [uint32]$SMTO_ABORTIFHUNG,
        [uint32]$MessageTimeoutMs, [ref]$result)
    Assert-UiOwnedWindow $Window
    if (-not $sent) {
        throw [System.TimeoutException]::new('OWNED_MESSAGE_TIMEOUT')
    }
    return $result.ToInt64()
}

function Get-UiOwnedSettingsComboSelection(
    [IntPtr]$SettingsDialog,
    [int]$ControlId,
    [int]$ExpectedIndex,
    [string]$ExpectedText
) {
    if ($SettingsDialog -eq [IntPtr]::Zero -or $ControlId -le 0 -or $ExpectedIndex -lt 0 -or
        [string]::IsNullOrEmpty($ExpectedText)) {
        throw 'SETUP_FAILURE: invalid owned SettingsGUI ComboBox readback request.'
    }
    Assert-UiOwnedWindow $SettingsDialog
    [uint32]$dialogPid=0
    $dialogThread=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($SettingsDialog,[ref]$dialogPid)
    if ($dialogThread -eq 0 -or $dialogPid -ne $script:ProcessId -or
        [MdliteRustAcceptanceNative]::WindowClass($SettingsDialog) -cne 'MDLiteSettings' -or
        -not [MdliteRustAcceptanceNative]::IsWindowVisible($SettingsDialog) -or
        -not [MdliteRustAcceptanceNative]::IsWindowEnabled($SettingsDialog)) {
        throw 'ENVIRONMENT_BLOCKED: SettingsGUI dialog identity changed before ComboBox readback.'
    }
    try {
        $control=Get-UiControlById -Root $SettingsDialog -ControlId $ControlId `
            -Class 'COMBOBOX' -RequireVisible -RequireEnabled
    } catch [System.NotSupportedException] {
        throw [System.InvalidOperationException]::new(('PRODUCT_ASSERTION: required SettingsGUI ComboBox control {0} is unavailable.' -f $ControlId),$_.Exception)
    }
    if ($null -eq $control) { throw 'OBSERVER_FAILURE: owned SettingsGUI ComboBox control snapshot is missing before readback.' }
    foreach ($name in @('Handle','Id','Class','Visible','Enabled')) {
        $property=$control.PSObject.Properties[$name]
        if ($null -eq $property -or $null -eq $property.Value) {
            throw 'OBSERVER_FAILURE: owned SettingsGUI ComboBox metadata is incomplete before readback.'
        }
    }
    Assert-UiOwnedWindow $control.Handle
    [uint32]$controlPid=0
    $controlThread=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($control.Handle,[ref]$controlPid)
    if ($control.Id -ne $ControlId -or $control.Class -ine 'COMBOBOX' -or
        -not [bool]$control.Visible -or -not [bool]$control.Enabled -or
        $controlThread -ne $dialogThread -or $controlPid -ne $script:ProcessId) {
        throw 'ENVIRONMENT_BLOCKED: SettingsGUI ComboBox HWND, class, ID, visibility, enabled state, or owner changed before readback.'
    }
    try {
        $selectedBefore=[int](Send-UiOwnedMessage -Window $control.Handle -Message 0x0147)
    } catch [TimeoutException] {
        throw [TimeoutException]::new('ENVIRONMENT_BLOCKED: bounded owned SettingsGUI ComboBox selection read timed out.',$_.Exception)
    }
    if ($selectedBefore -ne $ExpectedIndex) {
        throw ('PRODUCT_ASSERTION: SettingsGUI ComboBox index before text readback expected {0}, got {1}.' -f $ExpectedIndex,$selectedBefore)
    }
    # ComboBox WM_GETTEXT returns the selection field; OwnedControlText provides its bounded cross-process read.
    try {
        $text=[MdliteRustAcceptanceNative]::OwnedControlText($control.Handle,[uint32]$script:ProcessId)
    } catch [TimeoutException] {
        throw [TimeoutException]::new('ENVIRONMENT_BLOCKED: bounded owned SettingsGUI ComboBox WM_GETTEXT timed out.',$_.Exception)
    } catch [System.InvalidOperationException] {
        if ($_.Exception.Message -match 'ownership') {
            throw [System.InvalidOperationException]::new(('ENVIRONMENT_BLOCKED: bounded owned SettingsGUI ComboBox text read lost identity: ' + $_.Exception.Message),$_.Exception)
        }
        throw [System.InvalidOperationException]::new(('OBSERVER_FAILURE: bounded owned SettingsGUI ComboBox text response was invalid: ' + $_.Exception.Message),$_.Exception)
    }
    if ($null -eq $text) { throw 'OBSERVER_FAILURE: owned SettingsGUI ComboBox text response is missing.' }
    Assert-UiOwnedWindow $SettingsDialog
    if ($dialogThread -ne [MdliteRustAcceptanceNative]::GetWindowThreadProcessId($SettingsDialog,[ref]$dialogPid) -or
        $dialogPid -ne $script:ProcessId -or
        [MdliteRustAcceptanceNative]::WindowClass($SettingsDialog) -cne 'MDLiteSettings' -or
        -not [MdliteRustAcceptanceNative]::IsWindowVisible($SettingsDialog) -or
        -not [MdliteRustAcceptanceNative]::IsWindowEnabled($SettingsDialog)) {
        throw 'ENVIRONMENT_BLOCKED: SettingsGUI dialog owner or state changed during ComboBox text readback.'
    }
    try {
        $controlAfter=Get-UiControlById -Root $SettingsDialog -ControlId $ControlId `
            -Class 'COMBOBOX' -RequireVisible -RequireEnabled
    } catch [System.NotSupportedException] {
        throw [System.InvalidOperationException]::new(('ENVIRONMENT_BLOCKED: SettingsGUI ComboBox control {0} disappeared during readback.' -f $ControlId),$_.Exception)
    }
    if ($null -eq $controlAfter) { throw 'OBSERVER_FAILURE: owned SettingsGUI ComboBox control snapshot is missing after readback.' }
    foreach ($name in @('Handle','Id','Class','Visible','Enabled')) {
        $property=$controlAfter.PSObject.Properties[$name]
        if ($null -eq $property -or $null -eq $property.Value) {
            throw 'OBSERVER_FAILURE: owned SettingsGUI ComboBox metadata is incomplete after readback.'
        }
    }
    Assert-UiOwnedWindow $controlAfter.Handle
    [uint32]$controlPidAfter=0
    $controlThreadAfter=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($controlAfter.Handle,[ref]$controlPidAfter)
    if ($controlAfter.Handle -ne $control.Handle -or $controlAfter.Id -ne $ControlId -or
        $controlAfter.Class -ine 'COMBOBOX' -or -not [bool]$controlAfter.Visible -or
        -not [bool]$controlAfter.Enabled -or $controlThreadAfter -ne $dialogThread -or
        $controlPidAfter -ne $script:ProcessId) {
        throw 'ENVIRONMENT_BLOCKED: SettingsGUI ComboBox HWND, class, ID, visibility, enabled state, or owner changed after readback.'
    }
    try {
        $selectedAfter=[int](Send-UiOwnedMessage -Window $controlAfter.Handle -Message 0x0147)
    } catch [TimeoutException] {
        throw [TimeoutException]::new('ENVIRONMENT_BLOCKED: bounded owned SettingsGUI ComboBox selection recheck timed out.',$_.Exception)
    }
    if ($selectedAfter -ne $selectedBefore -or $selectedAfter -ne $ExpectedIndex) {
        throw ('PRODUCT_ASSERTION: SettingsGUI ComboBox selection changed during text readback; before={0}, after={1}.' -f $selectedBefore,$selectedAfter)
    }
    if ($text -cne $ExpectedText) {
        throw ('PRODUCT_ASSERTION: SettingsGUI ComboBox text expected {0}, got {1}.' -f $ExpectedText,$text)
    }
    return [pscustomobject]@{
        control_id=$ControlId; hwnd='0x{0:X}' -f $control.Handle.ToInt64()
        index=$selectedAfter; text=$text; owner_pid=[int]$controlPidAfter
    }
}

function Get-UiListCount([IntPtr]$ListWindow) {
    return [int](Send-UiOwnedMessage -Window $ListWindow -Message 0x018B)
}

function Find-UiListItemExact([IntPtr]$ListWindow, [string]$Value) {
    Assert-UiOwnedWindow $ListWindow
    return [MdliteRustAcceptanceNative]::FindListBoxItemExact(
        $ListWindow, $Value, 1000, [uint32]$SMTO_ABORTIFHUNG)
}

function Find-UiListItemContaining([IntPtr]$ListWindow, [string]$Needle) {
    Assert-UiOwnedWindow $ListWindow
    $count = Get-UiListCount $ListWindow
    for ($index = 0; $index -lt $count; $index++) {
        $text = [MdliteRustAcceptanceNative]::ListBoxItemText(
            $ListWindow, $index, 1000, [uint32]$SMTO_ABORTIFHUNG)
        if ($text.IndexOf($Needle, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            return $index
        }
    }
    return -1
}

function Click-UiListItem([IntPtr]$ForegroundWindow, [int]$ControlId, [int]$Index) {
    $control = Get-UiControlById -Root $ForegroundWindow -ControlId $ControlId `
        -Class 'LISTBOX' -RequireVisible -RequireEnabled
    $count = Get-UiListCount $control.Handle
    if ($Index -lt 0 -or $Index -ge $count) {
        throw [System.InvalidOperationException]::new('LIST_ITEM_INDEX_UNAVAILABLE')
    }
    if ($Index -gt 0) {
        [void](Send-UiOwnedMessage -Window $control.Handle -Message 0x0197 `
            -WParam ([IntPtr]$Index))
    }
    $item = [int[]][MdliteRustAcceptanceNative]::ListBoxItemRect(
        $control.Handle, $Index, 1000, [uint32]$SMTO_ABORTIFHUNG)
    $clientX = [int][Math]::Floor(($item[0] + $item[2]) / 2)
    $clientY = [int][Math]::Floor(($item[1] + $item[3]) / 2)
    [int]$screenX = 0
    [int]$screenY = 0
    $converted = [MdliteRustAcceptanceNative]::OwnedClientPointToScreen(
        $control.Handle, $script:ProcessId, $clientX, $clientY,
        [ref]$screenX, [ref]$screenY)
    if (-not $converted) {
        throw [System.InvalidOperationException]::new('LIST_ITEM_GEOMETRY_UNAVAILABLE')
    }
    Invoke-UiOwnedClick $ForegroundWindow $screenX $screenY
}

function DoubleClick-UiListItem([IntPtr]$ForegroundWindow, [int]$ControlId, [int]$Index) {
    Click-UiListItem $ForegroundWindow $ControlId $Index
    Start-Sleep -Milliseconds 90
    Click-UiListItem $ForegroundWindow $ControlId $Index
}

function Set-UiEditText(
    [IntPtr]$ForegroundWindow,
    [int]$ControlId,
    [string]$Value,
    [string]$Class = 'EDIT'
) {
    $control = Get-UiControlById -Root $ForegroundWindow -ControlId $ControlId `
        -Class $Class -RequireVisible -RequireEnabled
    Invoke-UiControlClick $ForegroundWindow $control
    Send-Key $ForegroundWindow 0x41 ([uint16[]]@(0x11))
    Send-Key $ForegroundWindow 0x08
    if ($Value.Length -gt 0) { Send-UnicodeText $Value $ForegroundWindow }
}

function Get-UiThreadWindows {
    Assert-UiOwnedWindow $script:WindowHandle
    foreach ($handleValue in [MdliteRustAcceptanceNative]::OwnedThreadWindows(
        $script:WindowHandle, $script:ProcessId)) {
        $handle = [IntPtr]([long]$handleValue)
        if ([MdliteRustAcceptanceNative]::IsWindowVisible($handle)) {
            [pscustomobject]@{
                Handle = $handle
                Class = [MdliteRustAcceptanceNative]::WindowClass($handle)
                Title = [MdliteRustAcceptanceNative]::WindowTitle($handle)
            }
        }
    }
}

function Wait-UiThreadWindow(
    [string]$Class,
    [string]$Title = '',
    [int]$WaitMs = 3000
) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    do {
        $matches = @(
            Get-UiThreadWindows | Where-Object {
                $_.Class -ceq $Class -and
                ([string]::IsNullOrEmpty($Title) -or $_.Title -ceq $Title)
            }
        )
        if ($matches.Count -eq 1) { return $matches[0].Handle }
        Start-Sleep -Milliseconds 50
        $script:Process.Refresh()
        if ($script:Process.HasExited) { break }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw [System.NotSupportedException]::new('OWNED_DIALOG_UNAVAILABLE')
}

function Wait-UiWindowGone([IntPtr]$Window, [int]$WaitMs = 2500) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    do {
        if (-not [MdliteRustAcceptanceNative]::IsWindow($Window)) { return }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw [System.TimeoutException]::new('OWNED_WINDOW_CLOSE_TIMEOUT')
}

function Invoke-UiDialogButton([IntPtr]$Dialog, [int]$ButtonId) {
    Invoke-UiControlById -ForegroundWindow $Dialog -ControlId $ButtonId -Class 'BUTTON'
}

function Wait-UiListCount(
    [IntPtr]$Root,
    [int]$ControlId,
    [int]$Minimum,
    [int]$WaitMs = 5000
) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    do {
        try {
            $list = Get-UiControlById -Root $Root -ControlId $ControlId `
                -Class 'LISTBOX' -RequireVisible -RequireEnabled
            $count = Get-UiListCount $list.Handle
            if ($count -ge $Minimum) { return $count }
        } catch [System.NotSupportedException] { }
        Start-Sleep -Milliseconds 80
    } while ([DateTime]::UtcNow -lt $deadline)
    throw [System.TimeoutException]::new('LIST_RESULT_COUNT_TIMEOUT')
}

function Invoke-UiPaletteCommand([string]$Label) {
    Invoke-UiControlById -ForegroundWindow $script:WindowHandle -ControlId 100
    $query = $null
    $results = $null
    $deadline = [DateTime]::UtcNow.AddMilliseconds(2500)
    do {
        try {
            $query = Get-UiControlById -Root $script:WindowHandle -ControlId 150 `
                -Class 'EDIT' -RequireVisible -RequireEnabled
            $results = Get-UiControlById -Root $script:WindowHandle -ControlId 151 `
                -Class 'LISTBOX' -RequireVisible -RequireEnabled
            break
        } catch [System.NotSupportedException] {
            Start-Sleep -Milliseconds 50
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($null -eq $query -or $null -eq $results) {
        throw [System.NotSupportedException]::new('PALETTE_CONTROLS_UNAVAILABLE')
    }
    Invoke-UiControlClick $script:WindowHandle $query
    $queryFocusReady = $false
    do {
        if ([DateTime]::UtcNow -ge $deadline) { break }
        try {
            $focusQuery = Get-UiControlById -Root $script:WindowHandle -ControlId 150 `
                -Class 'EDIT' -RequireVisible -RequireEnabled
            Assert-UiOwnedWindow $script:WindowHandle
            Assert-UiOwnedWindow $focusQuery.Handle
            $focusInfo = [MdliteRustAcceptanceNative]::ObserveOwnedEditorFocus(
                $script:WindowHandle, $script:WindowHandle, [IntPtr]::Zero, [uint32]$script:ProcessId)
            $focusQueryHandleText = '0x{0:X}' -f $focusQuery.Handle.ToInt64()
            $queryFocusReady = $focusInfo.registered_editor_window -and
                $focusInfo.gui_thread_info_available -and
                $focusInfo.foreground_is_expected -and $focusInfo.active_is_expected -and
                $focusInfo.focus_hwnd -ceq $focusQueryHandleText -and
                $focusInfo.caret_hwnd -ceq $focusQueryHandleText
            if ($queryFocusReady) { break }
        } catch [System.NotSupportedException] {
        }
        if ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 25 }
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $queryFocusReady) {
        throw [System.TimeoutException]::new('PALETTE_QUERY_FOCUS_TIMEOUT')
    }
    Send-UnicodeText $Label $script:WindowHandle
    $queryText = $null
    $count = -1
    $deadline = [DateTime]::UtcNow.AddMilliseconds(2500)
    do {
        try {
            $query = Get-UiControlById -Root $script:WindowHandle -ControlId 150 `
                -Class 'EDIT' -RequireVisible -RequireEnabled
            $results = Get-UiControlById -Root $script:WindowHandle -ControlId 151 `
                -Class 'LISTBOX' -RequireVisible -RequireEnabled
            Assert-UiOwnedWindow $query.Handle
            Assert-UiOwnedWindow $results.Handle
            $queryText = [MdliteRustAcceptanceNative]::OwnedControlText($query.Handle, [uint32]$script:ProcessId)
            $count = -1
            if ($queryText -ceq $Label) {
                $count = Get-UiListCount $results.Handle
                if ($count -eq 1) { break }
            }
        } catch [System.NotSupportedException] {
        } catch [System.TimeoutException] {
        }
        if ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($queryText -cne $Label) {
        throw [System.TimeoutException]::new('PALETTE_QUERY_TEXT_TIMEOUT')
    }
    if ($count -lt 0) {
        throw [System.TimeoutException]::new('PALETTE_RESULT_COUNT_TIMEOUT')
    }
    if ($count -ne 1) {
        throw [System.InvalidOperationException]::new('PALETTE_FILTER_NOT_UNIQUE')
    }
    Send-Key $script:WindowHandle 0x0D
    $closedDeadline = [DateTime]::UtcNow.AddMilliseconds(2500)
    do {
        $stillOpen = @(
            Get-UiOwnedChildren $script:WindowHandle | Where-Object {
                $_.Id -eq 150 -and $_.Class -ceq 'EDIT' -and $_.Visible
            }
        ).Count -gt 0
        if (-not $stillOpen) { return }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $closedDeadline)
    throw [System.TimeoutException]::new('PALETTE_COMMAND_TIMEOUT')
}

function Invoke-UiPaletteCancel([string]$Query) {
    $before = Read-Diagnostic
    Invoke-UiControlById -ForegroundWindow $script:WindowHandle -ControlId 100
    $query = Get-UiControlById -Root $script:WindowHandle -ControlId 150 `
        -Class 'EDIT' -RequireVisible -RequireEnabled
    Invoke-UiControlClick $script:WindowHandle $query
    Send-UnicodeText $Query $script:WindowHandle
    Send-Key $script:WindowHandle 0x1B
    $after = Wait-Diagnostic {
        param($state)
        [int]$state.document_id -eq [int]$before.document_id -and
        [int]$state.revision -eq [int]$before.revision -and
        [int]$state.selection.anchor -eq [int]$before.selection.anchor -and
        [int]$state.selection.head -eq [int]$before.selection.head
    } 'palette cancel preserving the current document state'
    if ($after.source -cne $before.source) {
        throw [System.InvalidOperationException]::new('PALETTE_CANCEL_CHANGED_SOURCE')
    }
}

function Get-UiParentRoot($State) {
    $property = $State.PSObject.Properties['parent_root']
    if ($null -eq $property -or $null -eq $property.Value) {
        throw [System.NotSupportedException]::new('PARENT_ROOT_METADATA_UNAVAILABLE')
    }
    $metadata = $property.Value
    foreach ($name in @('hwnd', 'dpi', 'window_rect', 'client_rect', 'panel_rects', 'features')) {
        if ($null -eq $metadata.PSObject.Properties[$name] -or
            $null -eq $metadata.PSObject.Properties[$name].Value) {
            throw [System.NotSupportedException]::new('PARENT_ROOT_METADATA_INCOMPLETE')
        }
    }
    $hwndText = [string]$metadata.hwnd
    [long]$reportedHwnd = 0
    if ($hwndText.StartsWith('0x', [StringComparison]::OrdinalIgnoreCase)) {
        $reportedHwnd = [Convert]::ToInt64($hwndText.Substring(2), 16)
    } else {
        $reportedHwnd = [long]$metadata.hwnd
    }
    if ($reportedHwnd -ne $script:WindowHandle.ToInt64()) {
        throw [System.InvalidOperationException]::new('PARENT_ROOT_HWND_MISMATCH')
    }
    $liveWindowRect = [int[]][MdliteRustAcceptanceNative]::WindowRect($script:WindowHandle)
    $liveClientRect = [int[]][MdliteRustAcceptanceNative]::ClientRect($script:WindowHandle)
    $edges = @('left', 'top', 'right', 'bottom')
    for ($index = 0; $index -lt $edges.Count; $index++) {
        $edge = $edges[$index]
        if ([int]$metadata.window_rect.$edge -ne $liveWindowRect[$index] -or
            [int]$metadata.client_rect.$edge -ne $liveClientRect[$index]) {
            throw [System.InvalidOperationException]::new('PARENT_ROOT_RECT_MISMATCH')
        }
    }
    $children = @(Get-UiOwnedChildren $script:WindowHandle)
    $script:Report.parent_root = [ordered]@{
        schema = [int]$metadata.schema
        hwnd = $hwndText
        editor_hwnd = [string]$metadata.editor_hwnd
        compact_hwnd = [string]$metadata.compact_hwnd
        dpi_scale = [double]$metadata.dpi
        dpi_raw = [int]$metadata.dpi_raw
        window_rect_px = $metadata.window_rect
        client_rect_px = $metadata.client_rect
        panel_ids = @($metadata.panel_rects | ForEach-Object { [string]$_.id })
        native_child_count = $children.Count
        visible_native_child_count = @($children | Where-Object { $_.Visible }).Count
        calendar_cell_count = @($metadata.features.calendar_cells).Count
        outline_row_count = @($metadata.features.outline_rows).Count
    }
    return $metadata
}

function Convert-DiagnosticWindowHandle($Value) {
    $text = [string]$Value
    if ($text.StartsWith('0x', [StringComparison]::OrdinalIgnoreCase)) {
        return [IntPtr]([Convert]::ToInt64($text.Substring(2), 16))
    }
    return [IntPtr]([long]$Value)
}

function Assert-EditorRecoveryMainIdentity(
    [IntPtr]$ExpectedMainWindow,
    [IntPtr]$ReportedMainWindow,
    [IntPtr]$ReportedEditorWindow,
    [string]$Phase
) {
    if ($ExpectedMainWindow -eq [IntPtr]::Zero -or
        $ReportedMainWindow -ne $ExpectedMainWindow -or
        $ReportedEditorWindow -ne $ExpectedMainWindow) {
        throw ('ENVIRONMENT_BLOCKED: EditorRecovery requires the exact owned Main HWND during {0}; expected=0x{1:X}, reported_main=0x{2:X}, reported_editor=0x{3:X}.' -f
            $Phase,$ExpectedMainWindow.ToInt64(),$ReportedMainWindow.ToInt64(),$ReportedEditorWindow.ToInt64())
    }
}

function Assert-EditorRecoveryWorkspaceAutoSaveDisabledSetup($Settings) {
    $configured = $null
    if ($Settings -is [Collections.IDictionary]) {
        if ($Settings.Contains('configured_auto_save')) { $configured = $Settings['configured_auto_save'] }
    } elseif ($null -ne $Settings) {
        $member = $Settings.PSObject.Properties['configured_auto_save']
        if ($null -ne $member) { $configured = $member.Value }
    }
    if ($configured -isnot [bool]) {
        throw 'ENVIRONMENT_BLOCKED: owned Workspace auto_save=false setup evidence is missing or malformed.'
    }
    if ($configured) {
        throw 'SETUP_FAILURE: owned Workspace auto_save must be configured false for the Core dirty-state oracle.'
    }
    return $true
}

function Assert-EditorRecoveryWorkspaceRootLine([string]$SummaryText, [string]$ExpectedRoot) {
    $workspaceLines = @([Regex]::Split([string]$SummaryText, '\r\n|\n|\r') | Where-Object {
        $_.StartsWith('Workspace:', [StringComparison]::Ordinal)
    })
    if ($workspaceLines.Count -ne 1 -or
        -not $workspaceLines[0].StartsWith('Workspace: ', [StringComparison]::Ordinal)) {
        throw [System.InvalidOperationException]::new('WORKSPACE_ROOT_READBACK_UNAVAILABLE')
    }
    $normalizeDrivePath = {
        param([string]$Path)
        if ([string]::IsNullOrWhiteSpace($Path)) {
            throw [System.InvalidOperationException]::new('WORKSPACE_ROOT_PATH_INVALID')
        }
        $value = $Path
        if ($value.StartsWith('\\?\', [StringComparison]::Ordinal)) {
            $value = $value.Substring(4)
        }
        if ($value -notmatch '^[A-Za-z]:\\') {
            throw [System.InvalidOperationException]::new('WORKSPACE_ROOT_PATH_INVALID')
        }
        $value = [IO.Path]::GetFullPath($value)
        $driveRoot = [IO.Path]::GetPathRoot($value)
        if ($value.Length -gt $driveRoot.Length) {
            $value = $value.TrimEnd([char]'\', [char]'/')
        }
        return $value
    }
    $reportedRoot = & $normalizeDrivePath $workspaceLines[0].Substring('Workspace: '.Length)
    $expectedNormalized = & $normalizeDrivePath $ExpectedRoot
    if (-not [string]::Equals($reportedRoot, $expectedNormalized, [StringComparison]::OrdinalIgnoreCase)) {
        throw [System.InvalidOperationException]::new('WORKSPACE_ROOT_READBACK_MISMATCH')
    }
    return $reportedRoot
}

function Get-EditorRecoveryWorkspaceRootReceipt([string]$ExpectedRoot) {
    Assert-UiOwnedWindow $script:WindowHandle
    $summary = Get-UiControlById -Root $script:WindowHandle -ControlId 6500 -Class 'EDIT'
    Assert-UiOwnedWindow $summary.Handle
    $summaryText = [MdliteRustAcceptanceNative]::OwnedControlText($summary.Handle, $script:ProcessId)
    $root = Assert-EditorRecoveryWorkspaceRootLine $summaryText $ExpectedRoot
    return [ordered]@{
        status = 'PASS'; root = $root; control_id = [int]$summary.Id
        control_class = [string]$summary.Class; owner_pid = [int]$script:ProcessId
    }
}

function Invoke-EditorViewportBodyClick([string]$CaseId, [string]$Phase) {
    $record = [pscustomobject][ordered]@{
        case_id = $CaseId; phase = $Phase; stage = $null; observed_utc = $null
        pid = [int]$script:ProcessId
        main_hwnd = '0x{0:X}' -f $script:WindowHandle.ToInt64()
        click_request_attempted = $false; click_call_returned = $false
        expected_window_kind = 'Main'
        click_viewport_dip = $null; expected_editor_hwnd = $null; editor_hwnd_after_click = $null
        source_sha256_before = $null; source_sha256_after = $null
        document_id_unchanged = $null; revision_unchanged = $null; dirty_unchanged = $null
        history_unchanged = $null; selection_before = $null; selection_after = $null
        native_focus_before = $null; native_focus_after = $null; input_observed_ms = $null
    }
    $script:Report.native_editor_focus_checks += $record
    $record.stage = 'read-before-click'
    try { $before = Read-Diagnostic }
    catch [TimeoutException] {
        $_.Exception.Data['EditorRecoveryBodyDiagnosticFailureKind'] = 'OBSERVER_FAILURE'
        $_.Exception.Data['EditorRecoveryBodyDiagnosticStage'] = $record.stage
        throw
    }
    $parent = Get-UiParentRoot $before
    $expectedEditor = Convert-DiagnosticWindowHandle $parent.editor_hwnd
    $main = Convert-DiagnosticWindowHandle $parent.hwnd
    $compact = Convert-DiagnosticWindowHandle $parent.compact_hwnd
    Assert-EditorRecoveryMainIdentity $script:WindowHandle $main $expectedEditor 'before viewport click'
    $record.expected_editor_hwnd = '0x{0:X}' -f $expectedEditor.ToInt64()
    if ($script:InputWindowHandle -ne $script:WindowHandle) {
        throw 'ENVIRONMENT_BLOCKED: EditorRecovery Main viewport input HWND differs from the owned Main HWND.'
    }
    $focusBefore = [MdliteRustAcceptanceNative]::ObserveOwnedEditorFocus(
        $expectedEditor, $main, $compact, $script:ProcessId)
    $body = $parent.editor_rect
    if ([double]$body.width -le 0 -or [double]$body.height -le 0) {
        throw 'SETUP_FAILURE: owned editor viewport body has no clickable area.'
    }
    $x = [double]$body.x + ([double]$body.width / 2.0)
    $y = [double]$body.y + ([double]$body.height / 2.0)
    $record.click_viewport_dip = @{ x=$x; y=$y; dpi=[double]$parent.dpi }
    $record.stage = 'click-request'
    $record.click_request_attempted = $true
    Invoke-UiDiagnosticClick $x $y ([double]$parent.dpi)

    $record.stage = 'read-after-click'
    $record.click_call_returned = $true
    try { $after = Read-Diagnostic }
    catch [TimeoutException] {
        $_.Exception.Data['EditorRecoveryBodyDiagnosticFailureKind'] = 'OBSERVER_FAILURE'
        $_.Exception.Data['EditorRecoveryBodyDiagnosticStage'] = $record.stage
        throw
    }
    $afterParent = Get-UiParentRoot $after
    $afterMain = Convert-DiagnosticWindowHandle $afterParent.hwnd
    $afterEditor = Convert-DiagnosticWindowHandle $afterParent.editor_hwnd
    $afterCompact = Convert-DiagnosticWindowHandle $afterParent.compact_hwnd
    Assert-EditorRecoveryMainIdentity $expectedEditor $afterMain $afterEditor 'after viewport click'
    $focusAfter = [MdliteRustAcceptanceNative]::ObserveOwnedEditorFocus(
        $expectedEditor, $afterMain, $afterCompact, $script:ProcessId)
    $record.editor_hwnd_after_click = '0x{0:X}' -f $afterEditor.ToInt64()
    $record.source_sha256_before = Get-UiTextSha256 ([string]$before.source)
    $record.source_sha256_after = Get-UiTextSha256 ([string]$after.source)
    $record.document_id_unchanged = ([long]$before.document_id -eq [long]$after.document_id)
    $record.revision_unchanged = ([int]$before.revision -eq [int]$after.revision)
    $record.dirty_unchanged = ([bool]$before.dirty -eq [bool]$after.dirty)
    $record.history_unchanged = ([int]$before.undo_count -eq [int]$after.undo_count -and
        [int]$before.redo_count -eq [int]$after.redo_count)
    $record.selection_before = $before.selection; $record.selection_after = $after.selection
    $record.native_focus_before = $focusBefore; $record.native_focus_after = $focusAfter
    if ([string]$before.source -cne [string]$after.source -or
        -not $record.document_id_unchanged -or -not $record.revision_unchanged -or
        -not $record.dirty_unchanged -or -not $record.history_unchanged) {
        throw 'PRODUCT_ASSERTION: owned editor viewport click changed source, document, revision, dirty state, or edit history.'
    }
    if (-not $focusAfter.registered_editor_window -or -not $focusAfter.gui_thread_info_available) {
        throw 'OBSERVER_FAILURE: native focus/caret state was unavailable for the exact owned Main/Compact editor HWND.'
    }
    if (-not $focusAfter.foreground_is_expected) {
        throw 'ENVIRONMENT_BLOCKED: the owned Main/Compact editor HWND lost foreground after the viewport click.'
    }
    if (-not $focusAfter.focus_is_expected -or -not $focusAfter.caret_is_expected) {
        throw 'PRODUCT_ASSERTION: viewport click did not leave native focus and caret on the exact registered Main/Compact editor HWND.'
    }
    $record.observed_utc = [DateTime]::UtcNow.ToString('o')
    $record.stage = 'completed'
    return $after
}

function Get-UiPanelGeometry($ParentRoot, [string]$PanelId) {
    $matches = @($ParentRoot.panel_rects | Where-Object { [string]$_.id -ceq $PanelId })
    if ($matches.Count -ne 1 -or $null -eq $matches[0].rect) {
        throw [System.NotSupportedException]::new('PANEL_GEOMETRY_UNAVAILABLE')
    }
    return $matches[0]
}

function Set-UiActivePanelFromHeader([int]$HeaderControlId) {
    Invoke-UiControlById $script:WindowHandle $HeaderControlId
    $menu = Wait-UiThreadWindow '#32768' '' 1500
    Send-Key $script:WindowHandle 0x1B
    Wait-UiWindowGone $menu 1500
}

function Ensure-UiCompactCalendarPopup {
    $parent = Get-UiParentRoot (Read-Diagnostic)
    $clientHeightDip = ([double]$parent.client_rect.bottom - [double]$parent.client_rect.top) / [double]$parent.dpi
    $maximumAttempts = [int][Math]::Ceiling($clientHeightDip / 24.0) + 1
    for ($attempt = 0; $attempt -lt $maximumAttempts; $attempt++) {
        $popup = @(Get-UiOwnedChildren $script:WindowHandle | Where-Object {
            $_.Id -eq 4818 -and $_.Class -ceq 'BUTTON' -and $_.Visible -and $_.Enabled
        })
        $cells = @($parent.features.calendar_cells)
        if ($popup.Count -eq 1 -and $cells.Count -gt 0) {
            return $parent
        }
        $calendar = Get-UiPanelGeometry $parent 'calendar'
        if ([bool]$calendar.hidden -or $null -eq $calendar.body) {
            throw [System.NotSupportedException]::new('CALENDAR_BODY_GEOMETRY_UNAVAILABLE')
        }
        $beforeHeight = [double]$calendar.body.height
        Set-UiActivePanelFromHeader 131
        $resizeLabel = if ($popup.Count -eq 1) {
            '対象パネルを高くする'
        } else {
            '対象パネルを低くする'
        }
        Invoke-UiPaletteCommand $resizeLabel
        $parent = Get-UiParentRoot (Read-Diagnostic)
        $after = Get-UiPanelGeometry $parent 'calendar'
        if ([double]$after.body.height -eq $beforeHeight) {
            throw [System.InvalidOperationException]::new('CALENDAR_COMPACT_RESIZE_DID_NOT_CHANGE_GEOMETRY')
        }
    }
    throw [System.InvalidOperationException]::new('CALENDAR_COMPACT_POPUP_NOT_REACHED')
}

function Invoke-UiCompactCalendarCommand([int]$CalendarCommandIndex) {
    if ($CalendarCommandIndex -lt 0 -or $CalendarCommandIndex -gt 7) {
        throw [System.ArgumentOutOfRangeException]::new('CalendarCommandIndex')
    }
    Invoke-UiControlById $script:WindowHandle 4818
    $menu = Wait-UiThreadWindow '#32768' '' 1500
    Send-Key $script:WindowHandle 0x24
    for ($index = 0; $index -lt $CalendarCommandIndex; $index++) {
        Send-Key $script:WindowHandle 0x28
    }
    Send-Key $script:WindowHandle 0x0D
    Wait-UiWindowGone $menu 2000
}

function Invoke-UiDiagnosticClick([double]$DipX, [double]$DipY, [double]$DpiScale) {
    $clientX = [int][Math]::Round($DipX * $DpiScale)
    $clientY = [int][Math]::Round($DipY * $DpiScale)
    [int]$screenX = 0
    [int]$screenY = 0
    $converted = [MdliteRustAcceptanceNative]::OwnedClientPointToScreen(
        $script:WindowHandle, $script:ProcessId, $clientX, $clientY,
        [ref]$screenX, [ref]$screenY)
    if (-not $converted) {
        throw [System.InvalidOperationException]::new('DIAGNOSTIC_GEOMETRY_CONVERSION_FAILED')
    }
    Invoke-UiOwnedClick $script:WindowHandle $screenX $screenY
}

function Invoke-UiDiagnosticShiftClick([double]$DipX, [double]$DipY, [double]$DpiScale) {
    $clientX = [int][Math]::Round($DipX * $DpiScale)
    $clientY = [int][Math]::Round($DipY * $DpiScale)
    [int]$screenX = 0
    [int]$screenY = 0
    $converted = [MdliteRustAcceptanceNative]::OwnedClientPointToScreen(
        $script:WindowHandle, $script:ProcessId, $clientX, $clientY,
        [ref]$screenX, [ref]$screenY)
    if (-not $converted) {
        throw [System.InvalidOperationException]::new('DIAGNOSTIC_GEOMETRY_CONVERSION_FAILED')
    }
    Set-OwnedForeground -Window $script:WindowHandle
    $sent = [MdliteRustAcceptanceNative]::ShiftClickOwnedAt(
        $script:WindowHandle, $script:ProcessId, $screenX, $screenY)
    if ($sent -eq 2) { return }
    if ($sent -eq -1) { throw [System.InvalidOperationException]::new('OWNERSHIP_GATE: Shift-click foreground or process owner changed.') }
    if ($sent -eq -2) { throw [System.InvalidOperationException]::new('ENVIRONMENT_BLOCKED: Shift-click point is outside the exact owned Main window.') }
    if ($sent -eq -3) { throw [System.InvalidOperationException]::new('ENVIRONMENT_BLOCKED: owned Shift-click cursor positioning failed.') }
    if ($sent -eq -4) { throw [System.InvalidOperationException]::new('ENVIRONMENT_BLOCKED: Shift-click SendInput could not press Shift.') }
    throw [System.InvalidOperationException]::new('ENVIRONMENT_BLOCKED: Shift-click SendInput did not complete and release all keys/buttons.')
}

function Assert-UiDiagnosticDragInvariants($State, $Baseline, [string]$Stage) {
    foreach ($name in @('document_id','source','revision','dirty','undo_count','redo_count','raw','tabs','active_input_profile','ime')) {
        if ($null -eq $State.PSObject.Properties[$name] -or $null -eq $State.PSObject.Properties[$name].Value) {
            throw ('OBSERVER_FAILURE: drag {0} diagnostic lacks {1}.' -f $Stage,$name)
        }
    }
    if ([long]$State.document_id -ne [long]$Baseline.document_id -or
        [string]$State.source -cne [string]$Baseline.source -or
        [long]$State.revision -ne [long]$Baseline.revision -or
        [bool]$State.dirty -ne [bool]$Baseline.dirty -or
        [int]$State.undo_count -ne [int]$Baseline.undo_count -or
        [int]$State.redo_count -ne [int]$Baseline.redo_count -or
        [bool]$State.raw -ne [bool]$Baseline.raw -or
        [int]$State.tabs -ne [int]$Baseline.tabs -or
        (ConvertTo-Json -InputObject $State.active_input_profile -Depth 12 -Compress) -cne
            (ConvertTo-Json -InputObject $Baseline.active_input_profile -Depth 12 -Compress) -or
        (ConvertTo-Json -InputObject $State.ime -Depth 12 -Compress) -cne
            (ConvertTo-Json -InputObject $Baseline.ime -Depth 12 -Compress)) {
        throw ('PRODUCT_ASSERTION: drag {0} changed source/document/revision/dirty/history/tabs/raw/profile/IME state.' -f $Stage)
    }
}

function Assert-UiDiagnosticDragOwnershipWhileHeld([uint32]$ExpectedThreadId) {
    Assert-OwnedWindow $script:WindowHandle
    $foreground = [MdliteRustAcceptanceNative]::GetForegroundWindow()
    [uint32]$foregroundProcessId = 0
    if ($foreground -ne [IntPtr]::Zero) {
        [void][MdliteRustAcceptanceNative]::GetWindowThreadProcessId($foreground,[ref]$foregroundProcessId)
    }
    if ($foreground -ne $script:WindowHandle -or $foregroundProcessId -ne $script:ProcessId) {
        throw 'OWNERSHIP_GATE: exact owned Main foreground changed while drag button was held; refusing to refocus.'
    }

    [uint32]$targetProcessId = 0
    $targetThreadId = [MdliteRustAcceptanceNative]::GetWindowThreadProcessId(
        $script:WindowHandle,[ref]$targetProcessId)
    if ($targetThreadId -eq 0 -or $targetProcessId -ne $script:ProcessId -or
        $targetThreadId -ne $ExpectedThreadId) {
        throw 'OWNERSHIP_GATE: exact owned Main GUI-thread identity changed while drag button was held.'
    }

    $capture = [IntPtr]::Zero
    $captureError = 0
    $captureRead = [MdliteRustAcceptanceNative]::TryGetThreadCapture(
        $targetThreadId,[ref]$capture,[ref]$captureError)
    if (-not $captureRead -or $capture -ne $script:WindowHandle) {
        throw ('OWNERSHIP_GATE: exact owned Main GUI-thread capture changed while drag button was held; error={0}.' -f $captureError)
    }
    if (-not [MdliteRustAcceptanceNative]::IsLeftButtonDown()) {
        throw 'OWNERSHIP_GATE: physical left button was released before the held-drag condition was accepted.'
    }
}

function Invoke-UiDiagnosticDrag(
    [double]$StartDipX,
    [double]$StartDipY,
    [double]$EndDipX,
    [double]$EndDipY,
    [double]$DpiScale,
    [Parameter(Mandatory=$true)][scriptblock]$WhileHeldCondition
) {
    $primaryFailure = $null
    $cleanupFailures = [Collections.Generic.List[string]]::new()
    $baseline = $null
    $downAccepted = $false
    $upAttempted = $false
    $upSent = $false
    $targetThreadId = [uint32]0
    $releaseRead = $false
    $releasedCapture = [IntPtr]::Zero
    $releasedCaptureError = 0
    $captureReleased = $false
    $buttonStillDown = $null
    $buttonReleased = $false
    try {
        $startX = [int][Math]::Round($StartDipX * $DpiScale)
        $startY = [int][Math]::Round($StartDipY * $DpiScale)
        $endX = [int][Math]::Round($EndDipX * $DpiScale)
        $endY = [int][Math]::Round($EndDipY * $DpiScale)
        [int]$screenStartX = 0
        [int]$screenStartY = 0
        [int]$screenEndX = 0
        [int]$screenEndY = 0
        $startConverted = [MdliteRustAcceptanceNative]::OwnedClientPointToScreen(
            $script:WindowHandle, $script:ProcessId, $startX, $startY,
            [ref]$screenStartX, [ref]$screenStartY)
        $endConverted = [MdliteRustAcceptanceNative]::OwnedClientPointToScreen(
            $script:WindowHandle, $script:ProcessId, $endX, $endY,
            [ref]$screenEndX, [ref]$screenEndY)
        if (-not $startConverted -or -not $endConverted) {
            throw [System.InvalidOperationException]::new('DIAGNOSTIC_GEOMETRY_CONVERSION_FAILED')
        }

        Assert-OwnedWindow $script:WindowHandle
        Set-OwnedForeground -Window $script:WindowHandle
        $foreground = [MdliteRustAcceptanceNative]::GetForegroundWindow()
        [uint32]$foregroundProcessId = 0
        if ($foreground -ne [IntPtr]::Zero) {
            [void][MdliteRustAcceptanceNative]::GetWindowThreadProcessId($foreground,[ref]$foregroundProcessId)
        }
        if ($foreground -ne $script:WindowHandle -or $foregroundProcessId -ne $script:ProcessId) {
            throw 'OWNERSHIP_GATE: drag requires the exact owned Main HWND in the foreground.'
        }

        $baseline = Read-Diagnostic
        Assert-UiDiagnosticDragInvariants $baseline $baseline 'baseline'
        $virtualMetrics = [int[]][MdliteRustAcceptanceNative]::GetVirtualScreenMetrics()
        if ($virtualMetrics.Count -ne 4 -or $virtualMetrics[2] -le 1 -or $virtualMetrics[3] -le 1) {
            throw 'ENVIRONMENT_BLOCKED: virtual-screen metrics are invalid.'
        }

        $startMoveError = 0
        $startMoveSent = [MdliteRustAcceptanceNative]::MoveAbsoluteVirtual(
            $screenStartX,$screenStartY,$virtualMetrics[0],$virtualMetrics[1],
            $virtualMetrics[2],$virtualMetrics[3],[ref]$startMoveError)
        $cursorX = 0
        $cursorY = 0
        $cursorError = 0
        $cursorRead = [MdliteRustAcceptanceNative]::TryGetCursorPosition([ref]$cursorX,[ref]$cursorY,[ref]$cursorError)
        if ($startMoveSent -ne 1 -or -not $cursorRead -or $cursorX -ne $screenStartX -or $cursorY -ne $screenStartY) {
            throw ('ENVIRONMENT_BLOCKED: absolute start MOVE/readback failed; count={0}; error={1}; cursor_error={2}.' -f
                $startMoveSent,$startMoveError,$cursorError)
        }
        $beforeDown = Read-Diagnostic
        Assert-UiDiagnosticDragInvariants $beforeDown $baseline 'pre-down'

        if ([MdliteRustAcceptanceNative]::IsLeftButtonDown()) {
            throw 'ENVIRONMENT_BLOCKED: left mouse button was already down before the staged drag.'
        }
        Assert-OwnedWindow $script:WindowHandle
        if ([MdliteRustAcceptanceNative]::GetForegroundWindow() -ne $script:WindowHandle) {
            throw 'OWNERSHIP_GATE: owned Main HWND lost foreground before drag button-down.'
        }
        $downError = 0
        $downSent = [MdliteRustAcceptanceNative]::LeftButtonDown([ref]$downError)
        if ($downSent -eq 1) { $downAccepted = $true }
        if (-not $downAccepted) {
            throw ('ENVIRONMENT_BLOCKED: staged drag button-down was not accepted; error={0}.' -f $downError)
        }

        [uint32]$targetProcessId = 0
        $targetThreadId = [MdliteRustAcceptanceNative]::GetWindowThreadProcessId(
            $script:WindowHandle,[ref]$targetProcessId)
        if ($targetThreadId -eq 0 -or $targetProcessId -ne $script:ProcessId) {
            throw 'OWNERSHIP_GATE: drag target GUI thread no longer belongs to the owned Main process.'
        }
        $capture = [IntPtr]::Zero
        $captureError = 0
        $captureRead = $false
        $captureDeadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
        do {
            $capture = [IntPtr]::Zero
            $captureRead = [MdliteRustAcceptanceNative]::TryGetThreadCapture(
                $targetThreadId,[ref]$capture,[ref]$captureError)
            if ($captureRead -and $capture -eq $script:WindowHandle) { break }
            if ([DateTime]::UtcNow -lt $captureDeadline) { Start-Sleep -Milliseconds 10 }
        } while ([DateTime]::UtcNow -lt $captureDeadline)
        if (-not $captureRead -or $capture -ne $script:WindowHandle) {
            throw ('ENVIRONMENT_BLOCKED: Main GUI-thread capture was not observed before timeout; error={0}.' -f $captureError)
        }
        $endMoveError = 0
        $afterDown = Read-Diagnostic
        Assert-UiDiagnosticDragInvariants $afterDown $baseline 'post-down'
        Assert-UiDiagnosticDragOwnershipWhileHeld $targetThreadId
        $endMoveSent = [MdliteRustAcceptanceNative]::MoveAbsoluteVirtual(
            $screenEndX,$screenEndY,$virtualMetrics[0],$virtualMetrics[1],
            $virtualMetrics[2],$virtualMetrics[3],[ref]$endMoveError)
        $endCursorX = 0
        $endCursorY = 0
        $endCursorError = 0
        $endCursorRead = [MdliteRustAcceptanceNative]::TryGetCursorPosition(
            [ref]$endCursorX,[ref]$endCursorY,[ref]$endCursorError)
        if ($endMoveSent -ne 1 -or -not $endCursorRead -or $endCursorX -ne $screenEndX -or $endCursorY -ne $screenEndY) {
            throw ('ENVIRONMENT_BLOCKED: absolute endpoint MOVE/readback failed; count={0}; error={1}; cursor_error={2}.' -f
                $endMoveSent,$endMoveError,$endCursorError)
        }

        $heldDeadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
        $conditionMatched = $false
        do {
            $heldState = Read-Diagnostic
            Assert-UiDiagnosticDragInvariants $heldState $baseline 'while-held'
            Assert-UiDiagnosticDragOwnershipWhileHeld $targetThreadId
            $conditionResults = @(& $WhileHeldCondition $heldState)
            if ($conditionResults.Count -ne 1 -or $conditionResults[0] -isnot [bool]) {
                throw 'WHILE_HELD_CONDITION_MUST_RETURN_EXACTLY_ONE_BOOLEAN'
            }
            if ($conditionResults[0]) { $conditionMatched = $true; break }
            if ([DateTime]::UtcNow -lt $heldDeadline) { Start-Sleep -Milliseconds 20 }
        } while ([DateTime]::UtcNow -lt $heldDeadline)
        if (-not $conditionMatched) {
            throw 'SEMANTIC_WAIT_TIMEOUT: drag WhileHeldCondition was not observed before button-up.'
        }
    } catch {
        $primaryFailure = $_
    } finally {
        if ($downAccepted) {
            $upAttempted = $true
            try {
                $upError = 0
                $upSentCount = [MdliteRustAcceptanceNative]::LeftButtonUp([ref]$upError)
                $upSent = ($upSentCount -eq 1)
                if (-not $upSent) { $cleanupFailures.Add(('LEFT_BUTTON_UP_NOT_ACCEPTED:{0}' -f $upError)) }
            } catch {
                $cleanupFailures.Add(('LEFT_BUTTON_UP_EXCEPTION:{0}' -f $_.Exception.Message))
            }

            try {
                $releaseDeadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
                do {
                    $releasedCapture = [IntPtr]::Zero
                    $releasedCaptureError = 0
                    $releaseRead = [MdliteRustAcceptanceNative]::TryGetThreadCapture(
                        $targetThreadId,[ref]$releasedCapture,[ref]$releasedCaptureError)
                    $buttonStillDown = [MdliteRustAcceptanceNative]::IsLeftButtonDown()
                    $captureReleased = ($releaseRead -and $releasedCapture -eq [IntPtr]::Zero)
                    $buttonReleased = -not $buttonStillDown
                    if ($captureReleased -and $buttonReleased) { break }
                    if ([DateTime]::UtcNow -lt $releaseDeadline) { Start-Sleep -Milliseconds 10 }
                } while ([DateTime]::UtcNow -lt $releaseDeadline)
                if (-not $captureReleased) { $cleanupFailures.Add('MAIN_GUI_THREAD_CAPTURE_NOT_RELEASED') }
                if (-not $buttonReleased) { $cleanupFailures.Add('LEFT_BUTTON_REMAINS_DOWN') }
            } catch {
                $cleanupFailures.Add(('RELEASE_OBSERVATION_EXCEPTION:{0}' -f $_.Exception.Message))
            }
        }

        if ($null -ne $baseline) {
            try {
                $afterUp = Read-Diagnostic
                Assert-UiDiagnosticDragInvariants $afterUp $baseline 'post-up'
            } catch {
                $cleanupFailures.Add(('POST_UP_DIAGNOSTIC_OR_INVARIANT_FAILURE:{0}' -f $_.Exception.Message))
            }
        }

        $cleanupStatus = if ($null -ne $primaryFailure) { 'CLEANUP_AFTER_PRIMARY_FAILURE' }
            elseif ($cleanupFailures.Count -gt 0) { 'FAIL' } else { 'PASS' }
        $cleanupReport = [ordered]@{
            status = $cleanupStatus
            primary_failure = if ($null -ne $primaryFailure) { [string]$primaryFailure.Exception.Message } else { $null }
            down_accepted = [bool]$downAccepted
            up_attempted = [bool]$upAttempted
            up_sent = [bool]$upSent
            release_capture_read = if ($downAccepted) { [bool]$releaseRead } else { $null }
            release_capture_hwnd = if ($downAccepted -and $releaseRead) { '0x{0:X}' -f $releasedCapture.ToInt64() } else { $null }
            capture_released = if ($downAccepted) { [bool]$captureReleased } else { $null }
            button_still_down = if ($downAccepted) { [bool]$buttonStillDown } else { $null }
            button_released = if ($downAccepted) { [bool]$buttonReleased } else { $null }
            secondary_failure_count = [int]$cleanupFailures.Count
            secondary_failures = @($cleanupFailures.ToArray() | Select-Object -First 8)
        }
        if ($script:Report -is [System.Collections.IDictionary]) {
            $script:Report['drag_cleanup'] = $cleanupReport
        } else {
            $cleanupFailures.Add('DRAG_CLEANUP_REPORT_NOT_DICTIONARY')
        }
    }

    if ($null -ne $primaryFailure) {
        throw $primaryFailure
    }
    if ($cleanupFailures.Count -gt 0) {
        throw [System.InvalidOperationException]::new('DRAG_CLEANUP_FAILURE: ' + ($cleanupFailures -join '; '))
    }
}

function Add-UiParityCase(
    [string]$Id,
    [string]$Status,
    [string]$EvidenceCode,
    $Evidence = $null
) {
    $script:Report.ui_parity_cases += [pscustomobject]@{
        id = $Id
        status = $Status
        evidence_code = $EvidenceCode
        evidence = $Evidence
    }
}

function Invoke-UiParityCase([string]$Id, [scriptblock]$Action) {
    $script:CurrentUiCaseId = $Id
    try {
        $evidence = & $Action
        Add-UiParityCase $Id 'PASS' 'OBSERVED' $evidence
        $script:CurrentUiCaseId = $null
        return $true
    } catch [System.NotSupportedException] {
        Add-UiParityCase $Id 'BLOCKED' $_.Exception.Message $null
        $script:CurrentUiCaseId = $null
        return $false
    }
}

function Complete-UiParityCaseList([string[]]$ExpectedIds) {
    $done = @($script:Report.ui_parity_cases | ForEach-Object { $_.id })
    foreach ($id in $ExpectedIds) {
        if ($done -cnotcontains $id) {
            Add-UiParityCase $id 'NOT_RUN' 'EARLIER_CASE_ABORTED' $null
        }
    }
}

function Wait-UiState([scriptblock]$Condition, [string]$Description, [int]$WaitMs = $TimeoutMs) {
    return Wait-Diagnostic $Condition $Description $WaitMs
}

function Wait-UiSource([string]$ExpectedSource, [bool]$ExpectedDirty, [int]$WaitMs = $TimeoutMs) {
    return Wait-UiState {
        param($state)
        $state.source -ceq $ExpectedSource -and [bool]$state.dirty -eq $ExpectedDirty
    } 'expected synthetic document source and dirty state' $WaitMs
}

function Switch-UiSource([string]$ExpectedSource, [int]$MaxTabs = 256) {
    $state = Read-Diagnostic
    if ($state.source -ceq $ExpectedSource) { return $state }
    for ($step = 0; $step -lt $MaxTabs; $step++) {
        $oldDocumentId = [long]$state.document_id
        $parentProperty = $state.PSObject.Properties['parent_root']
        if ($null -ne $parentProperty -and $null -ne $parentProperty.Value.editor_rect) {
            $editorRect = $parentProperty.Value.editor_rect
            $x = [double]$editorRect.x + [Math]::Min(12.0, [double]$editorRect.width / 4.0)
            $y = [double]$editorRect.y + [Math]::Min(12.0, [double]$editorRect.height / 4.0)
            Invoke-UiDiagnosticClick $x $y ([double]$parentProperty.Value.dpi)
        }
        Send-Key $script:InputWindowHandle 0x09 ([uint16[]]@(0x11))
        $state = Wait-UiState {
            param($value)
            [long]$value.document_id -ne $oldDocumentId -or $value.source -ceq $ExpectedSource
        } 'bounded Ctrl+Tab navigation to a synthetic fixture' 2500
        if ($state.source -ceq $ExpectedSource) { return $state }
    }
    throw [System.InvalidOperationException]::new('SYNTHETIC_TAB_NOT_FOUND')
}

function Get-UiSafeFileHash([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-UiTextSha256([string]$Text) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text)
        return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Capture-UiOwnedWindow([IntPtr]$Window, [string]$Path) {
    Assert-UiOwnedWindow $Window
    Set-OwnedForeground -Window $Window
    $bounds = [int[]][MdliteRustAcceptanceNative]::WindowRect($Window)
    $width = $bounds[2] - $bounds[0]
    $height = $bounds[3] - $bounds[1]
    if ($width -lt 1 -or $height -lt 1 -or $width -gt 7680 -or $height -gt 4320) {
        throw [System.InvalidOperationException]::new('OWNED_WINDOW_CAPTURE_BOUNDS_INVALID')
    }
    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::new($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen(
            $bounds[0], $bounds[1], 0, 0,
            [System.Drawing.Size]::new($width, $height),
            [System.Drawing.CopyPixelOperation]::SourceCopy)
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

function Wait-UiFileState(
    [string]$Path,
    [string]$ExpectedHash,
    [bool]$ExpectedAbsent = $false,
    [int]$WaitMs = 5000
) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    do {
        $exists = Test-Path -LiteralPath $Path -PathType Leaf
        if ($ExpectedAbsent -and -not $exists) { return $true }
        if (-not $ExpectedAbsent -and $exists -and (Get-UiSafeFileHash $Path) -ceq $ExpectedHash) {
            return $true
        }
        Start-Sleep -Milliseconds 80
    } while ([DateTime]::UtcNow -lt $deadline)
    throw [System.TimeoutException]::new('SYNTHETIC_FILE_STATE_TIMEOUT')
}

function New-UiParityFixture {
    $runId = [string]$script:Report.run_id
    $script:UiRunDirectory = Join-Path $RunRoot $runId
    $script:UiFixtureRoot = Join-Path $script:UiRunDirectory 'workspace'
    $script:UiLocalAppData = Join-Path $script:UiRunDirectory 'local-appdata'
    [void][IO.Directory]::CreateDirectory($script:UiFixtureRoot)
    [void][IO.Directory]::CreateDirectory((Join-Path $script:UiFixtureRoot '.mdlite\.state'))
    [void][IO.Directory]::CreateDirectory((Join-Path $script:UiFixtureRoot '.mdlite\.cache'))
    [void][IO.Directory]::CreateDirectory($script:UiLocalAppData)
    [IO.File]::WriteAllText(
        (Join-Path $script:UiFixtureRoot '.gitignore'),
        ".mdlite/.state/`n.mdlite/.cache/`n",
        [Text.UTF8Encoding]::new($false))
    $gitTemplate = Join-Path $script:UiRunDirectory 'empty-git-template'
    $gitHooks = Join-Path $script:UiFixtureRoot '.mdlite/.cache/git-hooks'
    [void][IO.Directory]::CreateDirectory($gitTemplate)
    [void][IO.Directory]::CreateDirectory($gitHooks)

    $script:UiDocumentSources = @{}
    for ($index = 0; $index -lt 9; $index++) {
        $name = 'doc-{0:D2}.md' -f $index
        if ($index -eq 0) {
            $source = '# UI heading A' + [Environment]::NewLine +
                'alpha body' + [Environment]::NewLine + [Environment]::NewLine +
                '# UI heading B' + [Environment]::NewLine + 'beta body' + [Environment]::NewLine
        } elseif ($index -eq 1) {
            $source = 'disk-marker' + [Environment]::NewLine + 'workspace-marker' + [Environment]::NewLine
        } else {
            $source = ('tab-source-{0:D2}' -f $index) + [Environment]::NewLine
        }
        $path = Join-Path $script:UiFixtureRoot $name
        [IO.File]::WriteAllText($path, $source, [Text.UTF8Encoding]::new($false))
        $script:UiDocumentSources[$name] = $source
    }

    for ($index = 0; $index -lt 205; $index++) {
        $name = 'zz-quick-{0:D3}.md' -f $index
        $path = Join-Path $script:UiFixtureRoot $name
        $source = ('quick-target-{0:D3}' -f $index) + [Environment]::NewLine
        [IO.File]::WriteAllText($path, $source, [Text.UTF8Encoding]::new($false))
        $script:UiDocumentSources[$name] = $source
    }

    $script:UiWorkspaceSearchPath = Join-Path $script:UiFixtureRoot 'workspace-search.md'
    $script:UiReplacePath = Join-Path $script:UiFixtureRoot 'replace-fixture.md'
    $script:UiGitTargetPath = Join-Path $script:UiFixtureRoot 'git-target.md'
    $script:UiGitUnrelatedPath = Join-Path $script:UiFixtureRoot 'git-unrelated.md'
    $script:UiHolidayPath = Join-Path $script:UiFixtureRoot 'holiday-invalid.csv'
    [IO.File]::WriteAllText($script:UiWorkspaceSearchPath, "workspace-marker`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($script:UiReplacePath, "replace-token`nunchanged-line`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($script:UiGitTargetPath, "git baseline target`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($script:UiGitUnrelatedPath, "git baseline unrelated`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($script:UiHolidayPath, "not-a-date,Holiday`n", [Text.UTF8Encoding]::new($false))

    $git = (Get-Command git -ErrorAction Stop).Source
    & $git -C $script:UiFixtureRoot init --quiet ("--template={0}" -f $gitTemplate)
    if ($LASTEXITCODE -ne 0) { throw [System.InvalidOperationException]::new('FIXTURE_GIT_INIT_FAILED') }
    & $git -C $script:UiFixtureRoot config user.name 'MDLite UI acceptance fixture'
    if ($LASTEXITCODE -ne 0) { throw [System.InvalidOperationException]::new('FIXTURE_GIT_CONFIG_FAILED') }
    & $git -C $script:UiFixtureRoot config user.email 'mdlite-acceptance@example.invalid'
    if ($LASTEXITCODE -ne 0) { throw [System.InvalidOperationException]::new('FIXTURE_GIT_CONFIG_FAILED') }
    & $git -C $script:UiFixtureRoot config commit.gpgsign false
    if ($LASTEXITCODE -ne 0) { throw [System.InvalidOperationException]::new('FIXTURE_GIT_CONFIG_FAILED') }
    & $git -C $script:UiFixtureRoot config core.hooksPath $gitHooks
    if ($LASTEXITCODE -ne 0) { throw [System.InvalidOperationException]::new('FIXTURE_GIT_CONFIG_FAILED') }
    & $git -C $script:UiFixtureRoot add -- '.gitignore' 'git-target.md' 'git-unrelated.md'
    if ($LASTEXITCODE -ne 0) { throw [System.InvalidOperationException]::new('FIXTURE_GIT_ADD_FAILED') }
    & $git -C $script:UiFixtureRoot commit --quiet -m 'synthetic UI acceptance baseline'
    if ($LASTEXITCODE -ne 0) { throw [System.InvalidOperationException]::new('FIXTURE_GIT_COMMIT_FAILED') }
    [IO.File]::WriteAllText($script:UiGitTargetPath, "git target changed by fixture`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($script:UiGitUnrelatedPath, "unrelated worktree change`n", [Text.UTF8Encoding]::new($false))

    $script:UiInitialDocPaths = @(
        for ($index = 0; $index -lt 9; $index++) {
            Join-Path $script:UiFixtureRoot ('doc-{0:D2}.md' -f $index)
        }
    )
    $script:Report.fixture_counts = [ordered]@{
        startup_documents = 9
        duplicate_open_arguments = 1
        quick_open_candidates = 205
        git_fixture = 'local repository; no remote'
        workspace_file_count = @(Get-ChildItem -LiteralPath $script:UiFixtureRoot -File).Count
    }
}

function Assert-UiExecutableIdentity {
    if ($null -eq $script:Process -or $script:Process.HasExited) {
        throw [System.InvalidOperationException]::new('OWNED_PROCESS_NOT_RUNNING')
    }
    $script:Process.Refresh()
    $imagePath = [IO.Path]::GetFullPath($script:Process.MainModule.FileName)
    $expectedPath = [IO.Path]::GetFullPath($ExecutablePath)
    if (-not [string]::Equals($imagePath, $expectedPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw [System.InvalidOperationException]::new('PROCESS_IMAGE_PATH_MISMATCH')
    }
    $imageHash = (Get-FileHash -LiteralPath $imagePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($imageHash -cne [string]$ExpectedExecutableSha256.ToLowerInvariant()) {
        throw [System.InvalidOperationException]::new('PROCESS_IMAGE_HASH_MISMATCH')
    }
    $script:Report.process_image_path = [string]$script:Report.executable_path
    $script:Report.process_image_sha256 = $imageHash
    $script:Report.process_identity_matches_expected = $true
    $script:Report.process_identities += [pscustomobject]@{
        pid = [int]$script:Process.Id
        image_path = [string]$script:Report.executable_path
        image_sha256 = $imageHash
    }
}

function Start-UiParityProcess([string[]]$Paths, [string]$DiagnosticPath) {
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $ExecutablePath
    $startInfo.WorkingDirectory = $script:UiFixtureRoot
    $startInfo.UseShellExecute = $false
    $startInfo.EnvironmentVariables['LOCALAPPDATA'] = $script:UiLocalAppData
    $roamingFixture = Join-Path $script:UiLocalAppData 'Roaming'
    [IO.Directory]::CreateDirectory($roamingFixture) | Out-Null
    $startInfo.EnvironmentVariables['APPDATA'] = $roamingFixture
    $arguments = @($Paths | ForEach-Object { ConvertTo-WindowsArgument $_ })
    $arguments += '--diagnostic-path'
    $arguments += ConvertTo-WindowsArgument $DiagnosticPath
    $startInfo.Arguments = [string]::Join(' ', [string[]]$arguments)
    $script:Process = [Diagnostics.Process]::Start($startInfo)
    if ($null -eq $script:Process) {
        throw [System.InvalidOperationException]::new('OWNED_RELEASE_START_FAILED')
    }
    $script:ProcessId = [uint32]$script:Process.Id
    $script:Report.pid = [int]$script:ProcessId
    $script:Report['startup_window_timeout_ms'] = $StartupWindowTimeoutMs
    $script:WindowHandle = [IntPtr]::Zero
    $script:InputWindowHandle = [IntPtr]::Zero
    $script:DiagnosticPath = $DiagnosticPath
    $deadline = [DateTime]::UtcNow.AddMilliseconds($StartupWindowTimeoutMs)
    do {
        $script:Process.Refresh()
        if ($script:Process.HasExited) {
            throw [System.InvalidOperationException]::new('OWNED_RELEASE_EXITED_BEFORE_WINDOW')
        }
        $candidate = $script:Process.MainWindowHandle
        if ($candidate -ne [IntPtr]::Zero -and
            [MdliteRustAcceptanceNative]::WindowClass($candidate) -eq 'MDLiteRustEditor' -and
            [MdliteRustAcceptanceNative]::IsOwnedWindow($candidate, $script:ProcessId)) {
            $script:WindowHandle = $candidate
            break
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($script:WindowHandle -eq [IntPtr]::Zero) {
        throw [System.TimeoutException]::new('OWNED_NATIVE_WINDOW_TIMEOUT')
    }
    $script:InputWindowHandle = $script:WindowHandle
    Assert-UiExecutableIdentity
    $script:Report.pid = [int]$script:ProcessId
    $script:Report.hwnd = '0x{0:X}' -f $script:WindowHandle.ToInt64()
    $script:Report.visible = [MdliteRustAcceptanceNative]::IsWindowVisible($script:WindowHandle)
    $script:Report.dpi = [int][MdliteRustAcceptanceNative]::GetDpiForWindow($script:WindowHandle)
    if (-not $script:Report.visible -or $script:Report.dpi -le 0) {
        throw [System.InvalidOperationException]::new('OWNED_NATIVE_WINDOW_NOT_VISIBLE')
    }
    Set-OwnedForeground -Window $script:WindowHandle -WaitMs 60000
}

function Invoke-UiParitySuite {
    if ($UiParity -and $StartupSmoke) {
        throw 'Use either -UiParity or -StartupSmoke, not both.'
    }
    if ([string]::IsNullOrWhiteSpace($ExpectedExecutableSha256) -or
        $ExpectedExecutableSha256 -notmatch '^[0-9a-fA-F]{64}$') {
        throw 'This owned GUI run requires -ExpectedExecutableSha256 from the selected build.'
    }
    $expectedBuildPattern = if ($StartupSmoke) {
        '[\\/]target[\\/]debug[\\/]mdlite\.exe$'
    } else {
        '[\\/]target[\\/]release[\\/]mdlite\.exe$'
    }
    if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf) -or
        $ExecutablePath -notmatch $expectedBuildPattern) {
        $buildLabel = if ($StartupSmoke) { 'target/debug/mdlite.exe' } else { 'target/release/mdlite.exe' }
        throw "This harness mode accepts only $buildLabel."
    }

    $runId = '{0}-{1}' -f [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ'),
        [guid]::NewGuid().ToString('N')
    $script:Report.schema_version = 2
    $script:Report.status = 'RUNNING'
    $script:Report.configuration = if ($StartupSmoke) { 'Debug' } else { 'Release' }
    $script:Report.executable_path = if ($StartupSmoke) { 'target/debug/mdlite.exe' } else { 'target/release/mdlite.exe' }
    $script:Report.harness_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $uiReportFields = [ordered]@{
        suite=if ($StartupSmoke) { 'StartupSmoke' } else { 'UiParity' }
        run_id=$runId
        expected_executable_sha256=$ExpectedExecutableSha256.ToLowerInvariant()
        harness_sha256_after=$null
        process_image_path=$null; process_image_sha256=$null; process_identity_matches_expected=$false
        appdata_isolated=$true
        appdata_isolation_scope='LOCALAPPDATA and APPDATA point to owned synthetic directories; the harness does not read host AppData.'
        parent_root=$null; initial_window_capture=$null; fixture_counts=$null
        process_identities=@(); ui_parity_cases=@()
        final_proof_valid=$true; executable_identity_changed=$false; source_tree_changed=$false; harness_changed=$false
        proof_invalidators=@(); recovery_restart=$null; failure_case_id=$null; failure_code=$null
    }
    foreach ($field in $uiReportFields.GetEnumerator()) {
        $script:Report.Add($field.Key, $field.Value)
    }
    $script:UiRunDirectory = Join-Path $RunRoot $runId
    [void][IO.Directory]::CreateDirectory($script:UiRunDirectory)
    $script:ReportPath = Join-Path $script:UiRunDirectory 'result.json'
    $script:DiagnosticPath = Join-Path $script:UiRunDirectory 'diagnostic.json'
    $script:ForegroundLogPath = $null
    $script:WindowHandle = [IntPtr]::Zero
    $script:InputWindowHandle = [IntPtr]::Zero
    $script:CurrentUiCaseId = $null

    $caseIds = @(
        'PARENT_ROOT_METADATA',
        'TABS_DUPLICATE_OPEN_SAVEAS_COLLISION',
        'SHELL_ACTIVITY_PANEL_SLOT_COLLAPSE_RESET_RESIZE_PALETTE_CANCEL',
        'SETTINGS_APPLY_CANCEL_KEYBINDING_REMAP',
        'EXPLORER_NEW_RENAME_INTERNAL_COPY_PASTE_OPEN_REFUSAL',
        'QUICK_OPEN_BLANK_LIMIT_AND_MORE',
        'QUICK_OPEN_LATE_FILE_QUERY_201_PLUS',
        'OUTLINE_JUMP_MOVE_SECTION_UNDO',
        'CALENDAR_SELECTED_DAILY_AND_PROFILE_CREATE',
        'CALENDAR_INVALID_HOLIDAY_CSV',
        'SEARCH_DIRTY_DOCUMENT_AND_WORKSPACE',
        'REPLACE_PREVIEW_AND_JOURNAL_RESTORE',
        'GIT_UNTRUSTED_GATE_AND_LOCAL_STAGE_UNSTAGE_COMMIT',
        'COMPACT_EDIT_UNDO_CLOSE_RETURN',
        'DIRTY_CLOSE_CANCEL_RESTART_RECOVERY_NORMAL_CLOSE'
    )
    if ($StartupSmoke) {
        $caseIds = @('STARTUP_VISIBLE_LAYOUT')
    }

    try {
        $script:Report.git_head = (& git -C $ProjectRoot rev-parse HEAD).Trim()
        if ($LASTEXITCODE -ne 0) { throw [System.InvalidOperationException]::new('GIT_HEAD_UNAVAILABLE') }
        $script:Report.executable_sha256_before = (Get-FileHash -LiteralPath $ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($script:Report.executable_sha256_before -cne $ExpectedExecutableSha256.ToLowerInvariant()) {
            throw [System.InvalidOperationException]::new('EXPECTED_RELEASE_HASH_MISMATCH')
        }
        $script:Report.source_tree_sha256_before = Get-SourceTreeSha256
        New-UiParityFixture
        $startupPaths = @($script:UiFixtureRoot) + @($script:UiInitialDocPaths) + @($script:UiInitialDocPaths[0])
        Start-UiParityProcess $startupPaths $script:DiagnosticPath
        $startupSources = @($script:UiDocumentSources.Values)
        $initial = Wait-UiState {
            param($state)
            [int]$state.tabs -eq 9 -and $startupSources -ccontains [string]$state.source
        } 'owned startup with nine unique tabs and one duplicate path' 15000
        $script:Report.tabs = [int]$initial.tabs
        $script:Report.active_input_profile = $initial.active_input_profile
        $script:UiCapturePath = Join-Path $script:UiRunDirectory 'initial-window.png'
        Capture-UiOwnedWindow $script:WindowHandle $script:UiCapturePath
        $script:Report.initial_window_capture = '.codex-runtime/editor-acceptance/' + $runId + '/initial-window.png'
        Add-Observation ($script:Report.suite + ' startup') $initial

        if ($StartupSmoke) {
            $runCaseIds = @('STARTUP_VISIBLE_LAYOUT')
            [void](Invoke-UiParityCase 'STARTUP_VISIBLE_LAYOUT' {
                $state = Read-Diagnostic
                $parent = Get-UiParentRoot $state
                $panelIds = @($parent.panel_rects | ForEach-Object { [string]$_.id } | Sort-Object -Unique)
                foreach ($required in @('explorer', 'calendar', 'outline', 'git')) {
                    if ($panelIds -cnotcontains $required) {
                        throw [System.InvalidOperationException]::new('PANEL_METADATA_MISSING')
                    }
                }
                if ([int]$script:Report.parent_root.native_child_count -le 0 -or
                    [double]$parent.dpi -le 0 -or
                    -not (Test-Path -LiteralPath $script:UiCapturePath -PathType Leaf)) {
                    throw [System.InvalidOperationException]::new('PARENT_ROOT_GEOMETRY_INVALID')
                }
                @{ tabs = [int]$state.tabs; panel_count = $panelIds.Count; child_count = [int]$script:Report.parent_root.native_child_count; own_window_capture = $true }
            })
            $closeResult = [IntPtr]::Zero
            $closeSent = [MdliteRustAcceptanceNative]::SendMessageTimeoutW(
                $script:WindowHandle, [uint32]$WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero,
                [uint32]$SMTO_ABORTIFHUNG, [uint32]1500, [ref]$closeResult)
            if (-not $closeSent -or -not $script:Process.WaitForExit(5000)) {
                throw [System.InvalidOperationException]::new('STARTUP_SMOKE_CLEAN_CLOSE_FAILED')
            }
            $script:Report.normal_close = 'PASS: clean owned startup smoke process exited.'
        } else {
            $runCaseIds = $caseIds
            [void](Invoke-UiParityCase 'PARENT_ROOT_METADATA' {
                $state = Read-Diagnostic
                $parent = Get-UiParentRoot $state
                $panelIds = @($parent.panel_rects | ForEach-Object { [string]$_.id } | Sort-Object -Unique)
                foreach ($required in @('explorer', 'calendar', 'outline', 'git')) {
                    if ($panelIds -cnotcontains $required) {
                        throw [System.InvalidOperationException]::new('PANEL_METADATA_MISSING')
                    }
                }
                if ([int]$script:Report.parent_root.native_child_count -le 0 -or
                    [double]$parent.dpi -le 0) {
                    throw [System.InvalidOperationException]::new('PARENT_ROOT_GEOMETRY_INVALID')
                }
                @{ tabs = [int]$state.tabs; panel_count = $panelIds.Count; child_count = [int]$script:Report.parent_root.native_child_count }
            })

        [void](Invoke-UiParityCase 'TABS_DUPLICATE_OPEN_SAVEAS_COLLISION' {
            $state = Read-Diagnostic
            if ([int]$state.tabs -ne 9) {
                throw [System.InvalidOperationException]::new('DUPLICATE_OPEN_DID_NOT_REUSE_TAB')
            }
            $doc0 = [string]$script:UiDocumentSources['doc-00.md']
            $doc1Path = Join-Path $script:UiFixtureRoot 'doc-01.md'
            $targetHash = Get-UiSafeFileHash $doc1Path
            [void](Switch-UiSource $doc0 9)
            Invoke-UiPaletteCommand '名前を付けて保存…'
            $dialog = $null
            try {
                $dialog = Wait-UiThreadWindow '#32770' '' 2500
            } catch [System.NotSupportedException] {
                throw [System.NotSupportedException]::new('SAVE_AS_DIALOG_UNAVAILABLE')
            }
            try {
                $nameField = Get-UiControlById -Root $dialog -ControlId 1001 `
                    -Class 'EDIT' -RequireVisible -RequireEnabled
                Set-UiEditText $dialog 1001 $doc1Path
                Send-Key $dialog 0x0D
                Start-Sleep -Milliseconds 300
                $confirmation = @(
                    Get-UiThreadWindows | Where-Object { $_.Class -ceq '#32770' -and $_.Handle -ne $dialog }
                )
                if ($confirmation.Count -gt 0) {
                    $buttons = @(Get-UiOwnedChildren $confirmation[0].Handle | Where-Object { $_.Class -ceq 'BUTTON' })
                    if (@($buttons | Where-Object { $_.Id -eq 7 }).Count -eq 1) {
                        Invoke-UiDialogButton $confirmation[0].Handle 7
                    } else {
                        Send-Key $confirmation[0].Handle 0x1B
                    }
                    Wait-UiWindowGone $confirmation[0].Handle 2500
                }
                if ([MdliteRustAcceptanceNative]::IsWindow($dialog)) {
                    Send-Key $dialog 0x1B
                    Wait-UiWindowGone $dialog 2500
                }
            } catch [System.NotSupportedException] {
                if ([MdliteRustAcceptanceNative]::IsWindow($dialog)) {
                    try { Send-Key $dialog 0x1B } catch { }
                }
                throw [System.NotSupportedException]::new('SAVE_AS_FILENAME_CONTROL_UNAVAILABLE')
            }
            Set-OwnedForeground -Window $script:WindowHandle
            $after = Wait-UiState {
                param($value)
                $value.source -ceq $doc0 -and [int]$value.tabs -eq 9
            } 'Save As collision preserving the open source and tab count'
            if ((Get-UiSafeFileHash $doc1Path) -cne $targetHash) {
                throw [System.InvalidOperationException]::new('SAVE_AS_OVERWROTE_OPEN_TARGET')
            }
            @{ tabs = [int]$after.tabs; target_unchanged = $true; save_as_dialog_observed = $true }
        })

        [void](Invoke-UiParityCase 'SHELL_ACTIVITY_PANEL_SLOT_COLLAPSE_RESET_RESIZE_PALETTE_CANCEL' {
            $state = Read-Diagnostic
            $parent = Get-UiParentRoot $state
            foreach ($activityId in @(110, 111, 112, 113)) {
                Invoke-UiControlById $script:WindowHandle $activityId
                switch ($activityId) {
                    110 { [void](Get-UiControlById $script:WindowHandle 4800 'BUTTON' -RequireVisible) }
                    111 { [void](Get-UiControlById $script:WindowHandle 5602 'EDIT' -RequireVisible) }
                    112 { [void](Get-UiControlById $script:WindowHandle 6000 'BUTTON' -RequireVisible) }
                    113 {
                        $calendarButtons = @(Get-UiOwnedChildren $script:WindowHandle | Where-Object {
                            $_.Class -ceq 'BUTTON' -and $_.Visible -and $_.Enabled -and $_.Id -in @(4812, 4818)
                        })
                        if ($calendarButtons.Count -ne 1) {
                            throw [System.NotSupportedException]::new('CALENDAR_OPERATION_ENTRY_UNAVAILABLE')
                        }
                    }
                }
            }
            Invoke-UiPaletteCancel 'no-matching-command-ui-parity'

            $initialPanel = Get-UiPanelGeometry $parent 'explorer'
            $initialSlotKey = ([string]$initialPanel.slot).ToLowerInvariant().Replace('_', '').Replace('-', '')
            Invoke-UiControlById $script:WindowHandle 130
            try { [void](Wait-UiThreadWindow '#32768' '' 1000) } catch [System.NotSupportedException] {
                throw [System.NotSupportedException]::new('PANEL_HEADER_MENU_UNAVAILABLE')
            }
            Send-Key $script:WindowHandle 0x1B
            $targetSlot = if ($initialSlotKey -eq 'righttop') { 'leftTop' } else { 'rightTop' }
            $moveLabel = if ($targetSlot -ceq 'rightTop') { '対象パネルを右上へ' } else { '対象パネルを左上へ' }
            Invoke-UiPaletteCommand $moveLabel
            $afterMove = Get-UiParentRoot (Read-Diagnostic)
            $movedPanel = Get-UiPanelGeometry $afterMove 'explorer'
            if (([string]$movedPanel.slot).ToLowerInvariant().Replace('_', '').Replace('-', '') -cne
                $targetSlot.ToLowerInvariant()) {
                throw [System.InvalidOperationException]::new('PANEL_SLOT_MOVE_NOT_OBSERVED')
            }
            Invoke-UiPaletteCommand 'パネル配置をリセット'
            $afterReset = Get-UiParentRoot (Read-Diagnostic)
            $resetPanel = Get-UiPanelGeometry $afterReset 'explorer'
            if (([string]$resetPanel.slot).ToLowerInvariant().Replace('_', '').Replace('-', '') -cne $initialSlotKey) {
                throw [System.InvalidOperationException]::new('PANEL_RESET_NOT_OBSERVED')
            }

            Invoke-UiControlById $script:WindowHandle 130
            [void](Wait-UiThreadWindow '#32768' '' 1000)
            Send-Key $script:WindowHandle 0x24
            Send-Key $script:WindowHandle 0x0D
            $collapsed = Wait-UiState {
                param($value)
                $panel = @($value.parent_root.panel_rects | Where-Object { [string]$_.id -ceq 'explorer' })[0]
                [bool]$panel.collapsed
            } 'Explorer panel collapse from its native header menu'
            if (-not [bool](@($collapsed.parent_root.panel_rects | Where-Object { [string]$_.id -ceq 'explorer' })[0].collapsed)) {
                throw [System.InvalidOperationException]::new('PANEL_COLLAPSE_NOT_OBSERVED')
            }
            $collapsedPanel = @($collapsed.parent_root.panel_rects | Where-Object { [string]$_.id -ceq 'explorer' })[0]
            Invoke-UiControlById $script:WindowHandle 130
            [void](Wait-UiThreadWindow '#32768' '' 1000)
            Send-Key $script:WindowHandle 0x24
            Send-Key $script:WindowHandle 0x0D
            $expanded = Wait-UiState {
                param($value)
                $panel = @($value.parent_root.panel_rects | Where-Object { [string]$_.id -ceq 'explorer' })[0]
                -not [bool]$panel.collapsed
            } 'Explorer panel restored from its native header menu'
            $expandedParent = Get-UiParentRoot $expanded
            $resizePanel = Get-UiPanelGeometry $expandedParent 'explorer'
            if ([bool]$resizePanel.hidden) { throw [System.NotSupportedException]::new('EXPLORER_PANEL_HIDDEN') }
            $beforeWidth = [double]$resizePanel.rect.width
            $slot = [string]$resizePanel.slot
            $leftSide = $slot -match '^left'
            $splitX = if ($leftSide) {
                [double]$resizePanel.rect.x + $beforeWidth
            } else {
                [double]$resizePanel.rect.x
            }
            $splitY = [double]$resizePanel.rect.y + [double]$resizePanel.rect.height / 2.0
            $delta = if ($leftSide) { 36.0 } else { -36.0 }
            Invoke-UiDiagnosticDrag $splitX $splitY ($splitX + $delta) $splitY ([double]$expandedParent.dpi) `
                -WhileHeldCondition {
                    param($state)
                    $panels = @($state.parent_root.panel_rects | Where-Object { [string]$_.id -ceq 'explorer' })
                    if ($panels.Count -ne 1) { [bool]$false }
                    else { [bool]([double]$panels[0].rect.width -gt ($beforeWidth + 8.0)) }
                }
            $resized = Wait-UiState {
                param($value)
                $panel = @($value.parent_root.panel_rects | Where-Object { [string]$_.id -ceq 'explorer' })[0]
                [double]$panel.rect.width -gt ($beforeWidth + 8.0)
            } 'Explorer splitter drag using diagnostic panel geometry'
            $resizedPanel = @($resized.parent_root.panel_rects | Where-Object { [string]$_.id -ceq 'explorer' })[0]
            if ([int]$resized.revision -ne [int]$expanded.revision) {
                throw [System.InvalidOperationException]::new('PANEL_RESIZE_CHANGED_DOCUMENT_REVISION')
            }
            Invoke-UiPaletteCommand 'パネル配置をリセット'
            @{ slot_move = [string]$movedPanel.slot; collapsed = [bool]$collapsedPanel.collapsed; resized_width_dip = [double]$resizedPanel.rect.width; source_revision_stable = $true }
        })

        [void](Invoke-UiParityCase 'SETTINGS_APPLY_CANCEL_KEYBINDING_REMAP' {
            $settingsPath = Join-Path $script:UiLocalAppData 'MDLite/settings.toml'
            $baselineHash = Get-UiSafeFileHash $settingsPath
            Invoke-UiControlById $script:WindowHandle 114
            $settings = Wait-UiThreadWindow 'MDLiteSettings' '' 3000
            Set-UiEditText $settings 205 '19'
            Invoke-UiDialogButton $settings 211
            $afterCancelHash = Get-UiSafeFileHash $settingsPath
            if ($afterCancelHash -cne $baselineHash) {
                throw [System.InvalidOperationException]::new('SETTINGS_CANCEL_WROTE_COMMON_FILE')
            }

            Invoke-UiControlById $script:WindowHandle 114
            $settings = Wait-UiThreadWindow 'MDLiteSettings' '' 3000
            $autoSave = Get-UiControlById $settings 201 'COMBOBOX' -RequireVisible -RequireEnabled
            Invoke-UiControlClick $settings $autoSave
            Send-Key $settings 0x23
            if ((Send-UiOwnedMessage $autoSave.Handle 0x0147) -ne 2) {
                throw [System.InvalidOperationException]::new('SETTINGS_AUTOSAVE_COMBO_SELECTION_FAILED')
            }
            Invoke-UiDialogButton $settings 210
            $notice = Wait-UiThreadWindow '#32770' '設定' 3000
            Invoke-UiDialogButton $notice 1
            $settingsBytes = [IO.File]::ReadAllText($settingsPath, [Text.Encoding]::UTF8)
            if ($settingsBytes -notmatch '(?im)^\s*auto_save\s*=\s*false\s*$') {
                throw [System.InvalidOperationException]::new('SETTINGS_APPLY_NOT_PERSISTED')
            }
            Set-UiEditText $settings 208 'file.quickOpen=Ctrl+Shift+Q'
            Invoke-UiDialogButton $settings 210
            $notice = Wait-UiThreadWindow '#32770' '設定' 3000
            Invoke-UiDialogButton $notice 1
            Invoke-UiDialogButton $settings 211
            Send-Key $script:WindowHandle 0x51 ([uint16[]]@(0x11, 0x10))
            $quickList = Wait-UiState {
                param($value)
                $control = @(Get-UiOwnedChildren $script:WindowHandle | Where-Object { $_.Id -eq 4691 -and $_.Visible })
                $control.Count -eq 1
            } 'remapped Ctrl+Shift+Q opens Quick Open' 3000
            Send-Key $script:WindowHandle 0x1B
            @{ cancel_unchanged = $true; apply_persisted = $true; remapped_shortcut_opened_quick_open = $true; settings_file_sha256 = Get-UiSafeFileHash $settingsPath }
        })

        [void](Invoke-UiParityCase 'EXPLORER_NEW_RENAME_INTERNAL_COPY_PASTE_OPEN_REFUSAL' {
            Invoke-UiControlById $script:WindowHandle 110
            $explorerList = Get-UiControlById $script:WindowHandle 4690 'LISTBOX' -RequireVisible -RequireEnabled
            Invoke-UiControlById $script:WindowHandle 4801
            Set-UiEditText $script:WindowHandle 4701 'created-ui-parity.md'
            Send-Key $script:WindowHandle 0x0D
            $createdPath = Join-Path $script:UiFixtureRoot 'created-ui-parity.md'
            $createdDeadline = [DateTime]::UtcNow.AddMilliseconds(3000)
            do {
                if (Test-Path -LiteralPath $createdPath -PathType Leaf) { break }
                Start-Sleep -Milliseconds 80
            } while ([DateTime]::UtcNow -lt $createdDeadline)
            if (-not (Test-Path -LiteralPath $createdPath -PathType Leaf)) {
                throw [System.InvalidOperationException]::new('EXPLORER_NEW_FILE_NOT_CREATED')
            }
            $openCount = [int](Read-Diagnostic).tabs
            Send-Key $script:WindowHandle 0x57 ([uint16[]]@(0x11))
            [void](Wait-UiState { param($value) [int]$value.tabs -eq ($openCount - 1) } 'closing the new clean fixture tab' 3000)

            $explorerList = Get-UiControlById $script:WindowHandle 4690 'LISTBOX' -RequireVisible -RequireEnabled
            $createdIndex = Find-UiListItemExact $explorerList.Handle '  created-ui-parity.md'
            if ($createdIndex -lt 0) { throw [System.InvalidOperationException]::new('EXPLORER_CREATED_FILE_NOT_LISTED') }
            Click-UiListItem $script:WindowHandle 4690 $createdIndex
            Invoke-UiControlById $script:WindowHandle 4805
            Set-UiEditText $script:WindowHandle 4701 'renamed-ui-parity.md'
            Send-Key $script:WindowHandle 0x0D
            $renamedPath = Join-Path $script:UiFixtureRoot 'renamed-ui-parity.md'
            if (-not (Test-Path -LiteralPath $renamedPath -PathType Leaf) -or
                (Test-Path -LiteralPath $createdPath -PathType Leaf)) {
                throw [System.InvalidOperationException]::new('EXPLORER_RENAME_NOT_OBSERVED')
            }
            $renamedHash = Get-UiSafeFileHash $renamedPath
            $explorerList = Get-UiControlById $script:WindowHandle 4690 'LISTBOX' -RequireVisible -RequireEnabled
            $renamedIndex = Find-UiListItemExact $explorerList.Handle '  renamed-ui-parity.md'
            if ($renamedIndex -lt 0) { throw [System.InvalidOperationException]::new('EXPLORER_RENAMED_FILE_NOT_LISTED') }
            Click-UiListItem $script:WindowHandle 4690 $renamedIndex
            Invoke-UiControlById $script:WindowHandle 4803
            Invoke-UiControlById $script:WindowHandle 4804
            $sameHashFiles = @(Get-ChildItem -LiteralPath $script:UiFixtureRoot -File | Where-Object {
                (Get-UiSafeFileHash $_.FullName) -ceq $renamedHash
            })
            if ($sameHashFiles.Count -lt 2) {
                throw [System.InvalidOperationException]::new('EXPLORER_INTERNAL_COPY_NOT_OBSERVED')
            }
            $doc0 = Get-UiControlById $script:WindowHandle 4690 'LISTBOX' -RequireVisible -RequireEnabled
            $doc0Index = Find-UiListItemExact $doc0.Handle '  doc-00.md'
            if ($doc0Index -lt 0) { throw [System.InvalidOperationException]::new('EXPLORER_OPEN_DOCUMENT_NOT_LISTED') }
            Click-UiListItem $script:WindowHandle 4690 $doc0Index
            $renameButtons = @(Get-UiOwnedChildren $script:WindowHandle | Where-Object {
                $_.Class -ceq 'BUTTON' -and $_.Id -eq 4805 -and $_.Visible
            })
            if ($renameButtons.Count -ne 1 -or $renameButtons[0].Enabled) {
                throw [System.InvalidOperationException]::new('EXPLORER_OPEN_DOCUMENT_RENAME_NOT_REFUSED')
            }
            @{ new_file = (Test-Path -LiteralPath $renamedPath -PathType Leaf); internal_copy_count = $sameHashFiles.Count; open_document_rename_disabled = $true; os_clipboard_used = $false }
        })

        [void](Invoke-UiParityCase 'QUICK_OPEN_BLANK_LIMIT_AND_MORE' {
            Invoke-UiControlById $script:WindowHandle 110
            Invoke-UiControlById $script:WindowHandle 4800
            $query = Get-UiControlById $script:WindowHandle 4700 'EDIT' -RequireVisible -RequireEnabled
            Set-UiEditText $script:WindowHandle 4700 ''
            $count = Wait-UiListCount $script:WindowHandle 4691 200 15000
            if ($count -ne 200) {
                throw [System.InvalidOperationException]::new('QUICK_OPEN_DISPLAY_CAP_NOT_OBSERVED')
            }
            $state = Read-Diagnostic
            $parent = Get-UiParentRoot $state
            $quick = $parent.features.PSObject.Properties['quick_open']
            if ($null -eq $quick -or $null -eq $quick.Value) {
                Send-Key $script:WindowHandle 0x1B
                throw [System.NotSupportedException]::new('QUICK_OPEN_MORE_METADATA_UNAVAILABLE')
            }
            if (-not [bool]$quick.Value.open -or [bool]$quick.Value.busy -or
                [int]$quick.Value.shown_count -ne 200 -or -not [bool]$quick.Value.more -or
                -not [bool]$quick.Value.complete) {
                Send-Key $script:WindowHandle 0x1B
                throw [System.InvalidOperationException]::new('QUICK_OPEN_OVERFLOW_NOT_REPORTED')
            }
            Send-Key $script:WindowHandle 0x1B
            @{ shown_count = $count; more = [bool]$quick.Value.more; complete = [bool]$quick.Value.complete }
        })

        [void](Invoke-UiParityCase 'QUICK_OPEN_LATE_FILE_QUERY_201_PLUS' {
            Invoke-UiControlById $script:WindowHandle 110
            Invoke-UiControlById $script:WindowHandle 4800
            Set-UiEditText $script:WindowHandle 4700 'zz-quick-204.md'
            $count = Wait-UiListCount $script:WindowHandle 4691 1 15000
            $quickList = Get-UiControlById $script:WindowHandle 4691 'LISTBOX' -RequireVisible -RequireEnabled
            $index = Find-UiListItemExact $quickList.Handle 'zz-quick-204.md'
            if ($index -lt 0) { throw [System.InvalidOperationException]::new('QUICK_OPEN_LATE_FILE_NOT_FOUND') }
            DoubleClick-UiListItem $script:WindowHandle 4691 $index
            $expected = [string]$script:UiDocumentSources['zz-quick-204.md']
            $state = Wait-UiState {
                param($value)
                $value.source -ceq $expected
            } 'opening a uniquely queried item after the 201st workspace file'
            @{ listed_count = $count; selected_index = $index; source_match = $true }
        })

        [void](Invoke-UiParityCase 'OUTLINE_JUMP_MOVE_SECTION_UNDO' {
            Invoke-UiControlById $script:WindowHandle 110
            $doc0 = [string]$script:UiDocumentSources['doc-00.md']
            [void](Switch-UiSource $doc0 32)
            $state = Read-Diagnostic
            $parent = Get-UiParentRoot $state
            $rows = @($parent.features.outline_rows)
            $secondHeading = $rows | Where-Object { [int]$_.source_start -eq $doc0.IndexOf('# UI heading B', [StringComparison]::Ordinal) } | Select-Object -First 1
            if ($null -eq $secondHeading) {
                throw [System.NotSupportedException]::new('OUTLINE_ROW_GEOMETRY_UNAVAILABLE')
            }
            $x = [double]$secondHeading.x + [double]$secondHeading.width / 2.0
            $y = [double]$secondHeading.y + [double]$secondHeading.height / 2.0
            Invoke-UiDiagnosticClick $x $y ([double]$parent.dpi)
            $headingStart = [int]$secondHeading.source_start
            $jumped = Wait-UiState {
                param($value)
                $value.source -ceq $doc0 -and [int]$value.selection.head -eq $headingStart
            } 'Outline click jumps to the selected heading source range'
            Invoke-UiControlById $script:WindowHandle 4820
            $moved = Wait-UiState {
                param($value)
                $value.source.IndexOf('# UI heading B', [StringComparison]::Ordinal) -lt
                    $value.source.IndexOf('# UI heading A', [StringComparison]::Ordinal)
            } 'Outline section move-up changes the selected section order'
            if (-not [bool]$moved.dirty) { throw [System.InvalidOperationException]::new('OUTLINE_MOVE_DID_NOT_DIRTY_DOCUMENT') }
            Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
            $restored = Wait-UiSource $doc0 $false
            @{ jumped_to_heading = $true; moved_and_undone = $restored.source -ceq $doc0; undo_count = [int]$restored.undo_count }
        })

        [void](Invoke-UiParityCase 'CALENDAR_SELECTED_DAILY_AND_PROFILE_CREATE' {
            Invoke-UiControlById $script:WindowHandle 113
            $state = Read-Diagnostic
            $parent = Get-UiParentRoot $state
            $today = Get-Date
            $cell = @($parent.features.calendar_cells | Where-Object {
                [int]$_.year -eq $today.Year -and [int]$_.month -eq $today.Month -and [int]$_.day -eq $today.Day
            })
            if ($cell.Count -ne 1) { throw [System.NotSupportedException]::new('CALENDAR_TODAY_CELL_UNAVAILABLE') }
            $x = [double]$cell[0].x + [double]$cell[0].width / 2.0
            $y = [double]$cell[0].y + [double]$cell[0].height / 2.0
            $beforeTabs = [int]$state.tabs
            Invoke-UiDiagnosticClick $x $y ([double]$parent.dpi)
            $dailyFiles = Join-Path $script:UiFixtureRoot 'Dairy'
            $deadline = [DateTime]::UtcNow.AddMilliseconds(5000)
            do {
                $created = if (Test-Path -LiteralPath $dailyFiles -PathType Container) {
                    @(Get-ChildItem -LiteralPath $dailyFiles -Filter '*.md' -File -Recurse)
                } else { @() }
                if ($created.Count -gt 0) { break }
                Start-Sleep -Milliseconds 80
            } while ([DateTime]::UtcNow -lt $deadline)
            if ($created.Count -ne 1) { throw [System.InvalidOperationException]::new('CALENDAR_DAILY_NOT_CREATED') }
            $profilePath = Join-Path $script:UiFixtureRoot '.mdlite/profiles.toml'
            [void](Ensure-UiCompactCalendarPopup)
            Invoke-UiCompactCalendarCommand 7
            Invoke-UiControlById $script:WindowHandle 4822
            $fields = @(
                'ui_acceptance',
                'UI Acceptance',
                'Profiles/{{date:yyyy}}',
                '{{date:yyyyMMdd}}-{{topic}}.md',
                '',
                'sequence',
                'topic|Topic|required|demo'
            )
            for ($index = 0; $index -lt $fields.Count; $index++) {
                Set-UiEditText $script:WindowHandle (4710 + $index) $fields[$index]
            }
            Invoke-UiControlById $script:WindowHandle 4828
            if (-not (Test-Path -LiteralPath $profilePath -PathType Leaf)) {
                throw [System.InvalidOperationException]::new('PROFILE_CREATE_FILE_MISSING')
            }
            $profiles = [IO.File]::ReadAllText($profilePath, [Text.Encoding]::UTF8)
            if ($profiles -notmatch '(?m)^\s*id\s*=\s*"ui_acceptance"\s*$') {
                throw [System.InvalidOperationException]::new('PROFILE_CREATE_NOT_PERSISTED')
            }
            Invoke-UiPaletteCommand 'プロファイル管理'
            $managerButtons = @(Get-UiOwnedChildren $script:WindowHandle | Where-Object {
                $_.Class -ceq 'BUTTON' -and $_.Id -eq 4822 -and $_.Visible
            })
            if ($managerButtons.Count -ne 0) {
                throw [System.InvalidOperationException]::new('PROFILE_MANAGER_DID_NOT_CLOSE')
            }
            @{ selected_daily_created = $true; profile_created = $true; daily_file_count = $created.Count; tabs_before = $beforeTabs }
        })

        [void](Invoke-UiParityCase 'CALENDAR_INVALID_HOLIDAY_CSV' {
            $cachePath = Join-Path $script:UiFixtureRoot '.mdlite/.cache/holidays.csv'
            $beforeCache = Get-UiSafeFileHash $cachePath
            Invoke-UiControlById $script:WindowHandle 113
            $managerButtons = @(Get-UiOwnedChildren $script:WindowHandle | Where-Object {
                $_.Class -ceq 'BUTTON' -and $_.Id -eq 4822 -and $_.Visible
            })
            if ($managerButtons.Count -gt 0) { Invoke-UiPaletteCommand 'プロファイル管理' }
            [void](Ensure-UiCompactCalendarPopup)
            Invoke-UiCompactCalendarCommand 6
            $dialog = $null
            try { $dialog = Wait-UiThreadWindow '#32770' '' 3000 } catch [System.NotSupportedException] {
                throw [System.NotSupportedException]::new('HOLIDAY_FILE_DIALOG_UNAVAILABLE')
            }
            try {
                Set-UiEditText $dialog 1001 $script:UiHolidayPath
                Send-Key $dialog 0x0D
                $deadline = [DateTime]::UtcNow.AddMilliseconds(3000)
                do {
                    $still = [MdliteRustAcceptanceNative]::IsWindow($dialog)
                    if (-not $still) { break }
                    Send-Key $dialog 0x0D
                    Start-Sleep -Milliseconds 80
                } while ([DateTime]::UtcNow -lt $deadline)
                if ([MdliteRustAcceptanceNative]::IsWindow($dialog)) {
                    Send-Key $dialog 0x1B
                    throw [System.NotSupportedException]::new('HOLIDAY_DIALOG_PATH_ENTRY_UNAVAILABLE')
                }
            } catch [System.NotSupportedException] {
                if ([MdliteRustAcceptanceNative]::IsWindow($dialog)) {
                    try { Send-Key $dialog 0x1B } catch { }
                }
                throw
            }
            if ((Get-UiSafeFileHash $cachePath) -cne $beforeCache) {
                throw [System.InvalidOperationException]::new('INVALID_HOLIDAY_IMPORT_CHANGED_CACHE')
            }
            $notice = $null
            try {
                $notice = Wait-UiThreadWindow '#32770' '' 2500
            } catch [System.NotSupportedException] {
                Invoke-UiPaletteCommand 'パネル配置をリセット'
                throw [System.NotSupportedException]::new('HOLIDAY_IMPORT_ERROR_STATUS_NOT_OBSERVABLE')
            }
            $noticeTitle = [MdliteRustAcceptanceNative]::WindowTitle($notice)
            if ($noticeTitle -eq 'MDLite') { Send-Key $notice 0x0D } else { Invoke-UiDialogButton $notice 1 }
            Invoke-UiPaletteCommand 'パネル配置をリセット'
            @{ invalid_csv_rejected = $true; holiday_cache_unchanged = $true }
        })

        [void](Invoke-UiParityCase 'SEARCH_DIRTY_DOCUMENT_AND_WORKSPACE' {
            Invoke-UiControlById $script:WindowHandle 110
            $original = [string]$script:UiDocumentSources['doc-01.md']
            [void](Switch-UiSource $original 32)
            Send-Key $script:WindowHandle 0x23 ([uint16[]]@(0x11))
            Send-UnicodeText 'dirty-marker' $script:WindowHandle
            $dirtySource = $original + 'dirty-marker'
            [void](Wait-UiSource $dirtySource $true)

            Invoke-UiControlById $script:WindowHandle 111
            Invoke-UiControlById $script:WindowHandle 5600
            Set-UiEditText $script:WindowHandle 5602 'dirty-marker'
            $documentResultCount = Wait-UiListCount $script:WindowHandle 5612 1 7000
            Click-UiListItem $script:WindowHandle 5612 0
            $dirtySearchState = Wait-UiState {
                param($value)
                $value.source -ceq $dirtySource -and [bool]$value.dirty -and
                    ([int]$value.selection.anchor -ge $original.Length -or
                     [int]$value.selection.head -ge $original.Length)
            } 'document search navigates to an unsaved dirty-only match'
            Send-Key $script:WindowHandle 0x53 ([uint16[]]@(0x11))
            $savedDirtyState = Wait-UiSource $dirtySource $false
            $dirtyFilePath = Join-Path $script:UiFixtureRoot 'doc-01.md'
            if ((Get-UiSafeFileHash $dirtyFilePath) -cne (Get-UiTextSha256 $dirtySource)) {
                throw [System.InvalidOperationException]::new('DIRTY_DOCUMENT_SAVE_MISMATCH')
            }
            Invoke-UiControlById $script:WindowHandle 5009

            Invoke-UiControlById $script:WindowHandle 111
            Set-UiEditText $script:WindowHandle 5602 'workspace-marker'
            $workspaceResultCount = Wait-UiListCount $script:WindowHandle 5612 3 10000
            $results = Get-UiControlById $script:WindowHandle 5612 'LISTBOX' -RequireVisible -RequireEnabled
            $groupIndex = Find-UiListItemContaining $results.Handle 'workspace-search.md'
            if ($groupIndex -lt 0 -or $groupIndex + 1 -ge $workspaceResultCount) {
                throw [System.InvalidOperationException]::new('WORKSPACE_SEARCH_CLOSED_FILE_RESULT_MISSING')
            }
            Click-UiListItem $script:WindowHandle 5612 ($groupIndex + 1)
            $closedSearchState = Wait-UiState {
                param($value)
                $value.source -ceq "workspace-marker`n"
            } 'workspace search opens a result from a closed synthetic document'
            Invoke-UiControlById $script:WindowHandle 5009
            @{ dirty_document_hits = $documentResultCount; dirty_match_navigated = $true; saved_dirty_source = $true; workspace_result_rows = $workspaceResultCount; closed_document_opened = $closedSearchState.source -ceq "workspace-marker`n" }
        })

        [void](Invoke-UiParityCase 'REPLACE_PREVIEW_AND_JOURNAL_RESTORE' {
            Invoke-UiControlById $script:WindowHandle 111
            Set-UiEditText $script:WindowHandle 5602 'replace-token'
            Set-UiEditText $script:WindowHandle 5603 'restored-token'
            [void](Wait-UiListCount $script:WindowHandle 5612 1 10000)
            $originalReplace = "replace-token`nunchanged-line`n"
            $replaced = "restored-token`nunchanged-line`n"
            $replacePath = $script:UiReplacePath
            $originalHash = Get-UiTextSha256 $originalReplace
            $replacedHash = Get-UiTextSha256 $replaced
            Invoke-UiControlById $script:WindowHandle 5006
            $previewDialog = Wait-UiThreadWindow '#32770' 'Workspace置換の確認' 10000
            Invoke-UiDialogButton $previewDialog 6
            [void](Wait-UiFileState $replacePath $replacedHash $false 10000)

            Invoke-UiControlById $script:WindowHandle 5007
            $journalCount = Wait-UiListCount $script:WindowHandle 5612 1 5000
            Click-UiListItem $script:WindowHandle 5612 0
            Invoke-UiControlById $script:WindowHandle 5008
            $restoreDialog = Wait-UiThreadWindow '#32770' '置換journalの復旧確認' 10000
            Invoke-UiDialogButton $restoreDialog 6
            [void](Wait-UiFileState $replacePath $originalHash $false 10000)
            $restoredBytes = [IO.File]::ReadAllText($replacePath, [Text.Encoding]::UTF8)
            if ($restoredBytes -cne $originalReplace) {
                throw [System.InvalidOperationException]::new('JOURNAL_RESTORE_BYTES_MISMATCH')
            }
            @{ replace_preview_applied = $true; journal_count = $journalCount; journal_restored_original = $true; journal_restore_is_fixture_only = $true }
        })

        [void](Invoke-UiParityCase 'GIT_UNTRUSTED_GATE_AND_LOCAL_STAGE_UNSTAGE_COMMIT' {
            Invoke-UiControlById $script:WindowHandle 112
            $summary = Get-UiControlById $script:WindowHandle 6500 'EDIT' -RequireVisible
            $summaryText = [MdliteRustAcceptanceNative]::OwnedControlText($summary.Handle, $script:ProcessId)
            if ($summaryText.IndexOf('Untrusted', [StringComparison]::OrdinalIgnoreCase) -lt 0) {
                throw [System.InvalidOperationException]::new('GIT_UNTRUSTED_STATE_NOT_SHOWN')
            }
            $refresh = Get-UiControlById $script:WindowHandle 6000 'BUTTON' -RequireVisible
            if ($refresh.Enabled) {
                throw [System.InvalidOperationException]::new('GIT_REFRESH_ENABLED_BEFORE_TRUST')
            }
            $trustedGitBefore = Get-Command git -ErrorAction Stop
            Invoke-UiPaletteCommand 'Workspaceをこの端末で信頼…'
            $trustDialog = Wait-UiThreadWindow '#32770' '端末local Trust' 5000
            Invoke-UiDialogButton $trustDialog 6
            $trustPath = Join-Path $script:UiLocalAppData 'MDLite/trust.toml'
            $trustDeadline = [DateTime]::UtcNow.AddMilliseconds(3000)
            do {
                if (Test-Path -LiteralPath $trustPath -PathType Leaf) { break }
                Start-Sleep -Milliseconds 80
            } while ([DateTime]::UtcNow -lt $trustDeadline)
            if (-not (Test-Path -LiteralPath $trustPath -PathType Leaf)) {
                throw [System.InvalidOperationException]::new('LOCAL_TRUST_FIXTURE_NOT_WRITTEN')
            }
            Invoke-UiControlById $script:WindowHandle 6000
            $fileCount = Wait-UiListCount $script:WindowHandle 6501 2 10000
            $files = Get-UiControlById $script:WindowHandle 6501 'LISTBOX' -RequireVisible -RequireEnabled
            $targetIndex = Find-UiListItemContaining $files.Handle 'git-target.md'
            if ($targetIndex -lt 0) { throw [System.InvalidOperationException]::new('GIT_TARGET_NOT_IN_STATUS_LIST') }
            Click-UiListItem $script:WindowHandle 6501 $targetIndex

            $cachedBefore = @(& $trustedGitBefore.Source -C $script:UiFixtureRoot diff --cached --name-only)
            if ($LASTEXITCODE -ne 0 -or $cachedBefore.Count -ne 0) {
                throw [System.InvalidOperationException]::new('GIT_FIXTURE_INDEX_NOT_CLEAN')
            }
            Invoke-UiControlById $script:WindowHandle 6005
            $stageConfirm = Wait-UiThreadWindow '#32770' 'Git確認' 5000
            Invoke-UiDialogButton $stageConfirm 6
            $stageExpected = 'git-target.md'
            $stageDeadline = [DateTime]::UtcNow.AddMilliseconds(10000)
            do {
                $staged = @(& $trustedGitBefore.Source -C $script:UiFixtureRoot diff --cached --name-only)
                if ($LASTEXITCODE -eq 0 -and $staged.Count -eq 1 -and $staged[0].Trim() -ceq $stageExpected) { break }
                Start-Sleep -Milliseconds 100
            } while ([DateTime]::UtcNow -lt $stageDeadline)
            if ($staged.Count -ne 1 -or $staged[0].Trim() -cne $stageExpected) {
                throw [System.InvalidOperationException]::new('GIT_STAGE_NOT_SELECTED_ONLY')
            }

            Invoke-UiControlById $script:WindowHandle 6006
            $unstageConfirm = Wait-UiThreadWindow '#32770' 'Git確認' 5000
            Invoke-UiDialogButton $unstageConfirm 6
            $unstageDeadline = [DateTime]::UtcNow.AddMilliseconds(10000)
            do {
                $cachedAfterUnstage = @(& $trustedGitBefore.Source -C $script:UiFixtureRoot diff --cached --name-only)
                if ($LASTEXITCODE -eq 0 -and $cachedAfterUnstage.Count -eq 0) { break }
                Start-Sleep -Milliseconds 100
            } while ([DateTime]::UtcNow -lt $unstageDeadline)
            if ($cachedAfterUnstage.Count -ne 0) {
                throw [System.InvalidOperationException]::new('GIT_UNSTAGE_NOT_OBSERVED')
            }

            Invoke-UiControlById $script:WindowHandle 6005
            $stageConfirm = Wait-UiThreadWindow '#32770' 'Git確認' 5000
            Invoke-UiDialogButton $stageConfirm 6
            $stageDeadline = [DateTime]::UtcNow.AddMilliseconds(10000)
            do {
                $staged = @(& $trustedGitBefore.Source -C $script:UiFixtureRoot diff --cached --name-only)
                if ($LASTEXITCODE -eq 0 -and $staged.Count -eq 1 -and $staged[0].Trim() -ceq $stageExpected) { break }
                Start-Sleep -Milliseconds 100
            } while ([DateTime]::UtcNow -lt $stageDeadline)
            if ($staged.Count -ne 1 -or $staged[0].Trim() -cne $stageExpected) {
                throw [System.InvalidOperationException]::new('GIT_RESTAGE_NOT_OBSERVED')
            }
            $headBeforeCommit = (& $trustedGitBefore.Source -C $script:UiFixtureRoot rev-parse HEAD).Trim()
            if ($LASTEXITCODE -ne 0) { throw [System.InvalidOperationException]::new('GIT_HEAD_READ_FAILED') }
            Invoke-UiPaletteCommand 'Git: indexだけCommit…'
            Set-UiEditText $script:WindowHandle 6503 'ui-parity-local-commit'
            Invoke-UiControlById $script:WindowHandle 6506
            $commitConfirm = Wait-UiThreadWindow '#32770' 'Git確認' 5000
            Invoke-UiDialogButton $commitConfirm 6
            $commitDeadline = [DateTime]::UtcNow.AddMilliseconds(12000)
            do {
                $headAfterCommit = (& $trustedGitBefore.Source -C $script:UiFixtureRoot rev-parse HEAD).Trim()
                if ($LASTEXITCODE -eq 0 -and $headAfterCommit -cne $headBeforeCommit) { break }
                Start-Sleep -Milliseconds 100
            } while ([DateTime]::UtcNow -lt $commitDeadline)
            if ($headAfterCommit -ceq $headBeforeCommit) {
                throw [System.InvalidOperationException]::new('GIT_COMMIT_NOT_OBSERVED')
            }
            $committedFiles = @(& $trustedGitBefore.Source -C $script:UiFixtureRoot show --format= --name-only HEAD)
            if ($LASTEXITCODE -ne 0 -or @($committedFiles | Where-Object { $_.Trim() -eq 'git-target.md' }).Count -ne 1 -or
                @($committedFiles | Where-Object { $_.Trim() -eq 'git-unrelated.md' }).Count -ne 0) {
                throw [System.InvalidOperationException]::new('GIT_COMMIT_INCLUDED_UNSELECTED_FILE')
            }
            $cachedFinal = @(& $trustedGitBefore.Source -C $script:UiFixtureRoot diff --cached --name-only)
            $working = @(& $trustedGitBefore.Source -C $script:UiFixtureRoot status --porcelain)
            $remotes = @(& $trustedGitBefore.Source -C $script:UiFixtureRoot remote)
            if ($LASTEXITCODE -ne 0 -or $cachedFinal.Count -ne 0 -or $remotes.Count -ne 0 -or
                -not (@($working | Where-Object { $_.Contains('git-unrelated.md') }).Count -eq 1)) {
                throw [System.InvalidOperationException]::new('GIT_LOCAL_ONLY_FIXTURE_STATE_MISMATCH')
            }
            @{ untrusted_refresh_disabled = $true; stage_selected_only = $true; unstage_empty_index = $true; commit_selected_only = $true; remotes = 0; local_process_only = $true; status_entries = $fileCount }
        })

        [void](Invoke-UiParityCase 'COMPACT_EDIT_UNDO_CLOSE_RETURN' {
            Invoke-UiControlById $script:WindowHandle 110
            $source = [string]$script:UiDocumentSources['doc-02.md']
            [void](Switch-UiSource $source 64)
            Invoke-UiPaletteCommand 'Compactへ移す'
            $compactDeadline = [DateTime]::UtcNow.AddMilliseconds(3000)
            $compact = [IntPtr]::Zero
            do {
                $matches = @(
                    Get-UiThreadWindows | Where-Object {
                        $_.Class -ceq 'MDLiteRustEditor' -and $_.Handle -ne $script:WindowHandle -and
                        $_.Title.IndexOf('Compact', [StringComparison]::OrdinalIgnoreCase) -ge 0
                    }
                )
                if ($matches.Count -eq 1) { $compact = $matches[0].Handle; break }
                Start-Sleep -Milliseconds 50
            } while ([DateTime]::UtcNow -lt $compactDeadline)
            if ($compact -eq [IntPtr]::Zero) {
                throw [System.NotSupportedException]::new('COMPACT_WINDOW_UNAVAILABLE')
            }
            $script:InputWindowHandle = $compact
            Send-Key $compact 0x23 ([uint16[]]@(0x11))
            Send-UnicodeText 'compact-edit' $compact
            $editedSource = $source + 'compact-edit'
            [void](Wait-UiSource $editedSource $true)
            Send-Key $compact 0x5A ([uint16[]]@(0x11))
            $undone = Wait-UiSource $source $false
            [void](Send-UiOwnedMessage -Window $compact -Message ([uint32]$WM_CLOSE) -MessageTimeoutMs 1500)
            Wait-UiWindowGone $compact 3000
            $script:InputWindowHandle = $script:WindowHandle
            Set-OwnedForeground -Window $script:WindowHandle
            $returned = Wait-UiSource $source $false
            @{ compact_edit_undone = $undone.source -ceq $source; close_returned_to_main = $returned.source -ceq $source; process_id_unchanged = [int]$script:Process.Id -eq [int]$script:ProcessId }
        })

        [void](Invoke-UiParityCase 'DIRTY_CLOSE_CANCEL_RESTART_RECOVERY_NORMAL_CLOSE' {
            $script:InputWindowHandle = $script:WindowHandle
            Invoke-UiControlById $script:WindowHandle 110
            $source = [string]$script:UiDocumentSources['doc-08.md']
            [void](Switch-UiSource $source 64)
            Send-Key $script:WindowHandle 0x23 ([uint16[]]@(0x11))
            Send-UnicodeText 'restart-recovery-marker' $script:WindowHandle
            $recoverySource = $source + 'restart-recovery-marker'
            [void](Wait-UiSource $recoverySource $true)
            $sourcePath = Join-Path $script:UiFixtureRoot 'doc-08.md'
            $diskBeforeClose = Get-UiSafeFileHash $sourcePath

            $closeResult = [IntPtr]::Zero
            $closeSent = [MdliteRustAcceptanceNative]::SendMessageTimeoutW(
                $script:WindowHandle, [uint32]$WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero,
                [uint32]$SMTO_ABORTIFHUNG, [uint32]1000, [ref]$closeResult)
            if (-not $closeSent) { throw [System.TimeoutException]::new('DIRTY_CLOSE_MESSAGE_TIMEOUT') }
            $dirtyPrompt = Wait-OwnedConfirmation 3000
            if ($dirtyPrompt -eq [IntPtr]::Zero) {
                throw [System.InvalidOperationException]::new('DIRTY_CLOSE_PROMPT_MISSING')
            }
            Send-Key $dirtyPrompt 0x1B
            $afterCancel = Wait-UiSource $recoverySource $true
            if ((Get-UiSafeFileHash $sourcePath) -cne $diskBeforeClose) {
                throw [System.InvalidOperationException]::new('DIRTY_CLOSE_CANCEL_CHANGED_DISK')
            }

            $recoveryDirectory = Join-Path $script:UiFixtureRoot '.mdlite/.state/recovery'
            $recoveryDeadline = [DateTime]::UtcNow.AddMilliseconds(12000)
            do {
                $recoveryRecords = if (Test-Path -LiteralPath $recoveryDirectory -PathType Container) {
                    @(Get-ChildItem -LiteralPath $recoveryDirectory -File)
                } else { @() }
                if ($recoveryRecords.Count -gt 0) { break }
                Start-Sleep -Milliseconds 100
            } while ([DateTime]::UtcNow -lt $recoveryDeadline)
            if ($recoveryRecords.Count -eq 0) {
                throw [System.InvalidOperationException]::new('RECOVERY_SNAPSHOT_NOT_WRITTEN')
            }
            Assert-UiExecutableIdentity
            $oldPid = [int]$script:Process.Id
            $script:Process.Kill()
            if (-not $script:Process.WaitForExit(5000)) {
                throw [System.TimeoutException]::new('OWNED_RECOVERY_PROCESS_DID_NOT_EXIT')
            }
            $script:Report.recovery_restart = [ordered]@{
                forced_owned_fixture_exit = $true
                recovery_record_count_before_restart = $recoveryRecords.Count
                first_pid = $oldPid
            }
            $script:Process.Dispose()
            $restartDiagnostic = Join-Path $script:UiRunDirectory 'diagnostic-restart.json'
            Start-UiParityProcess @($script:UiFixtureRoot) $restartDiagnostic
            $recoveryPrompt = Wait-UiThreadWindow '#32770' '復旧候補' 12000
            Invoke-UiDialogButton $recoveryPrompt 6
            $recovered = Wait-UiState {
                param($value)
                $value.source -ceq $recoverySource -and [bool]$value.dirty
            } 'restarted process opens the explicit synthetic recovery candidate' 10000
            $script:Report.recovery_restart.second_pid = [int]$script:Process.Id
            $script:Report.recovery_restart.restored_document_dirty = [bool]$recovered.dirty

            $tabsBeforeDiscard = [int]$recovered.tabs
            Send-Key $script:WindowHandle 0x57 ([uint16[]]@(0x11))
            $discardPrompt = Wait-OwnedConfirmation 3000
            if ($discardPrompt -eq [IntPtr]::Zero) {
                throw [System.InvalidOperationException]::new('RECOVERED_DIRTY_TAB_CLOSE_PROMPT_MISSING')
            }
            Invoke-UiDialogButton $discardPrompt 7
            [void](Wait-UiState {
                param($value)
                [int]$value.tabs -lt $tabsBeforeDiscard -and -not [bool]$value.dirty
            } 'discarding the synthetic recovered tab before normal close' 5000)

            $closeResult = [IntPtr]::Zero
            $closeSent = [MdliteRustAcceptanceNative]::SendMessageTimeoutW(
                $script:WindowHandle, [uint32]$WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero,
                [uint32]$SMTO_ABORTIFHUNG, [uint32]1500, [ref]$closeResult)
            if (-not $closeSent -or -not $script:Process.WaitForExit(5000)) {
                throw [System.InvalidOperationException]::new('NORMAL_CLEAN_CLOSE_FAILED')
            }
            $script:Report.normal_close = 'PASS: clean owned Release process exited after recovery verification.'
            $script:Report.recovery_restart.dirty_close_cancel_preserved = $afterCancel.dirty
            @{ dirty_cancel_preserved = [bool]$afterCancel.dirty; recovery_records = $recoveryRecords.Count; restarted_source_restored = $recovered.source -ceq $recoverySource; normal_close = $true }
        })

        }
        Complete-UiParityCaseList $runCaseIds
        $failed = @($script:Report.ui_parity_cases | Where-Object { $_.status -eq 'FAIL' })
        $unfinished = @($script:Report.ui_parity_cases | Where-Object { $_.status -in @('BLOCKED', 'NOT_RUN') })
        if ($failed.Count -gt 0) {
            $script:Report.status = 'FAIL'
        } elseif ($unfinished.Count -gt 0) {
            $script:Report.status = 'INCOMPLETE'
        } else {
            $script:Report.status = 'PASS'
        }
    } catch {
        $script:Report.status = 'FAIL'
        $script:Report.failure_case_id = $script:CurrentUiCaseId
        $script:Report.failure_code = if ($_.Exception.Message -match '^[A-Z0-9_]+$') {
            $_.Exception.Message
        } else {
            $_.Exception.GetType().Name.ToUpperInvariant()
        }
        Complete-UiParityCaseList $caseIds
    } finally {
        $script:Report.executable_sha256_after = Get-UiSafeFileHash $ExecutablePath
        $script:Report.source_tree_sha256_after = Get-SourceTreeSha256
        $script:Report.harness_sha256_after = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $script:Report.executable_identity_changed =
            $script:Report.executable_sha256_after -cne $script:Report.executable_sha256_before -or
            $script:Report.executable_sha256_after -cne $ExpectedExecutableSha256.ToLowerInvariant()
        $script:Report.source_tree_changed =
            $script:Report.source_tree_sha256_after -cne $script:Report.source_tree_sha256_before
        $script:Report.harness_changed =
            $script:Report.harness_sha256_after -cne $script:Report.harness_sha256
        if ($script:Report.executable_identity_changed) {
            $script:Report.proof_invalidators += 'EXECUTABLE_IDENTITY_CHANGED'
        }
        if ($script:Report.source_tree_changed) {
            $script:Report.proof_invalidators += 'SOURCE_TREE_CHANGED'
        }
        if ($script:Report.harness_changed) {
            $script:Report.proof_invalidators += 'HARNESS_CHANGED'
        }
        $script:Report.final_proof_valid = $script:Report.proof_invalidators.Count -eq 0
        if (-not $script:Report.final_proof_valid) {
            $script:Report.status = 'FAIL'
            if ([string]::IsNullOrWhiteSpace([string]$script:Report.failure_code)) {
                $script:Report.failure_code = $script:Report.proof_invalidators[0] + '_DURING_RUN'
            }
        }
        if ($null -ne $script:Process) {
            try {
                $script:Process.Refresh()
                $script:Report.process_still_running = -not $script:Process.HasExited
                if ($script:Report.status -ne 'PASS' -and -not $script:Process.HasExited) {
                    $script:Report.cleanup_note = 'Failure leaves the exact owned fixture process open for inspection.'
                }
            } catch {
                $script:Report.process_still_running = $null
            }
            $script:Process.Dispose()
        } else {
            $script:Report.process_still_running = $false
        }
        $script:Report.ended_utc = [DateTime]::UtcNow.ToString('o')
        $script:Report.result_path = '.codex-runtime/editor-acceptance/' + $runId + '/result.json'
        $json = $script:Report | ConvertTo-Json -Depth 14
        [IO.File]::WriteAllText($script:ReportPath, $json, [Text.UTF8Encoding]::new($false))
    }

    if ($script:Report.status -eq 'PASS') {
        Write-Output ('PASS: {0}. Evidence: .codex-runtime/editor-acceptance/{1}/result.json' -f
            $script:Report.suite, $runId)
    } else {
        Write-Output ('{0}: {1}. Evidence: .codex-runtime/editor-acceptance/{2}/result.json' -f
            $script:Report.status, $script:Report.suite, $runId)
    }
}

function New-EditorRecoveryFixture {
    $script:EditorRecoveryNoWorkspaceRoot = Join-Path $script:UiRunDirectory 'no-workspace'
    $script:UiFixtureRoot = Join-Path $script:UiRunDirectory 'workspace'
    $script:UiLocalAppData = Join-Path $script:UiRunDirectory 'local-appdata'
    foreach ($directory in @(
        $script:EditorRecoveryNoWorkspaceRoot, $script:UiFixtureRoot,
        (Join-Path $script:UiFixtureRoot '.mdlite\.state'),
        (Join-Path $script:UiFixtureRoot '.mdlite\.cache'),
        $script:UiLocalAppData, (Join-Path $script:UiLocalAppData 'Roaming')
    )) { [void][IO.Directory]::CreateDirectory($directory) }
    $ignore = '.mdlite/.state/' + [Environment]::NewLine + '.mdlite/.cache/' + [Environment]::NewLine
    [IO.File]::WriteAllText((Join-Path $script:UiFixtureRoot '.gitignore'), $ignore, [Text.UTF8Encoding]::new($false))
    $script:EditorRecoveryBlankPath = Join-Path $script:UiFixtureRoot 'new-document.md'
    $script:EditorRecoveryWorkspacePath = Join-Path $script:UiFixtureRoot 'editor.md'
    $script:EditorRecoveryPointerPath = Join-Path $script:UiFixtureRoot 'pointer-fixture.md'
    $repositoryEditorFixture = Join-Path $ProjectRoot 'tests/fixtures/editor.md'
    $repositoryEditorAsset = Join-Path $ProjectRoot 'tests/fixtures/assets/fixture.svg'
    if (-not (Test-Path -LiteralPath $repositoryEditorFixture -PathType Leaf)) {
        throw 'SETUP_FAILURE: repository editor fixture is unavailable for E06/E07.'
    }
    if (-not (Test-Path -LiteralPath $repositoryEditorAsset -PathType Leaf)) {
        throw 'SETUP_FAILURE: repository editor fixture SVG is unavailable for E06/E07.'
    }
    Copy-Item -LiteralPath $repositoryEditorFixture -Destination $script:EditorRecoveryPointerPath
    $script:EditorRecoveryPointerAssetPath=Join-Path $script:UiFixtureRoot 'assets/fixture.svg'
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $script:EditorRecoveryPointerAssetPath))
    Copy-Item -LiteralPath $repositoryEditorAsset -Destination $script:EditorRecoveryPointerAssetPath
    $sourceFixtureHash = (Get-FileHash -LiteralPath $repositoryEditorFixture -Algorithm SHA256).Hash.ToLowerInvariant()
    $copiedFixtureHash = (Get-FileHash -LiteralPath $script:EditorRecoveryPointerPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $sourceAssetHash = (Get-FileHash -LiteralPath $repositoryEditorAsset -Algorithm SHA256).Hash.ToLowerInvariant()
    $copiedAssetHash = (Get-FileHash -LiteralPath $script:EditorRecoveryPointerAssetPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($sourceFixtureHash -cne $copiedFixtureHash -or $sourceAssetHash -cne $copiedAssetHash) {
        throw 'SETUP_FAILURE: owned pointer fixture or its SVG asset differs from tests/fixtures.'
    }
    $script:EditorRecoveryPointerSource = [IO.File]::ReadAllText($script:EditorRecoveryPointerPath, [Text.Encoding]::UTF8)
    $script:EditorRecoveryWorkspaceSettingsPath = Join-Path $script:UiFixtureRoot '.mdlite/settings.toml'
    $script:EditorRecoveryLf = [string][char]10
    $script:EditorRecoveryCrLf = [string][char]13 + [string][char]10
    $script:EditorRecoveryWorkspaceSource = (@(
        '# Synthetic Editor Recovery fixture', '', '', 'alpha beta 日本語', '',
        '> quoted line', '', '- item one', '', '| first | second |',
        '| --- | --- |', '| cell | value |', '', '',
        '| third | fourth |', '| --- | --- |', '| next | last |', '',
        '~~~text', 'literal **Markdown**', '~~~', '', 'tail sentinel', ''
    ) -join $script:EditorRecoveryCrLf)
    $workspaceSettings = "schema_version = 1`nauto_save = false`n"
    [IO.File]::WriteAllText($script:EditorRecoveryWorkspaceSettingsPath, $workspaceSettings, [Text.UTF8Encoding]::new($false))
    $script:Report.workspace_settings = [ordered]@{
        path = $script:EditorRecoveryWorkspaceSettingsPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
        scope = 'harness-owned workspace only'
        configured_auto_save = $false
        runtime_application_readback = 'auto_save runtime NOT_EXPOSED; sha256 covers fixture setup only.'
        sha256 = (Get-FileHash -LiteralPath $script:EditorRecoveryWorkspaceSettingsPath -Algorithm SHA256).Hash.ToLowerInvariant()
        workspace_root_readbacks = [ordered]@{}
    }
    [IO.File]::WriteAllText($script:EditorRecoveryBlankPath, '', [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($script:EditorRecoveryWorkspacePath, $script:EditorRecoveryWorkspaceSource, [Text.UTF8Encoding]::new($true))
    $script:EditorRecoveryWorkspaceInitialHash=(Get-FileHash -LiteralPath $script:EditorRecoveryWorkspacePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:EditorRecoveryFixturePaths = @(
        $script:EditorRecoveryBlankPath, $script:EditorRecoveryWorkspacePath,
        $script:EditorRecoveryPointerPath
    )
    $script:Report.fixtures = @(
        foreach ($fixture in $script:EditorRecoveryFixturePaths) {
            [pscustomobject]@{
                path = $fixture.Substring($ProjectRoot.Length).TrimStart('\').Replace('\', '/')
                role = if ($fixture -ceq $script:EditorRecoveryPointerPath) { 'byte-exact copy of tests/fixtures/editor.md for E06/E07' } else { 'owned EditorRecovery fixture' }
                initial_sha256 = (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant()
                initial_bytes = [IO.File]::ReadAllBytes($fixture).Length
            }
        }
    )
    $script:Report.editor_recovery_pointer_fixture = [ordered]@{
        source_path = $repositoryEditorFixture.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
        source_sha256 = $sourceFixtureHash
        copy_path = $script:EditorRecoveryPointerPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
        copy_sha256 = $copiedFixtureHash
        byte_exact = $sourceFixtureHash -ceq $copiedFixtureHash
        asset_source_path = $repositoryEditorAsset.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
        asset_copy_path = $script:EditorRecoveryPointerAssetPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
        asset_sha256 = $copiedAssetHash
        asset_byte_exact = $sourceAssetHash -ceq $copiedAssetHash
    }
    $script:Report.workspace_root = $script:UiFixtureRoot.Substring($ProjectRoot.Length).TrimStart('\').Replace('\', '/')
    $script:Report.appdata_root = $script:UiLocalAppData.Substring($ProjectRoot.Length).TrimStart('\').Replace('\', '/')
}

function Get-EditorRecoveryProfileIdentity($Profile) {
    if ($null -eq $Profile) { return $null }
    $identity = [ordered]@{}
    foreach ($name in @('status','description','profile_type','language_id','clsid','profile_guid','hkl')) {
        $property = $Profile.PSObject.Properties[$name]
        $identity[$name] = if ($null -eq $property) { $null } else { $property.Value }
    }
    return [pscustomobject]$identity
}

function Get-EditorRecoveryBytesSha256([byte[]]$Bytes) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}

function Test-EditorRecoveryJapaneseText([string]$Text) {
    return $Text -match '[\u3040-\u30FF\u3400-\u4DBF\u4E00-\u9FFF\uF900-\uFAFF\uFF66-\uFF9D]'
}

function Wait-EditorRecoveryStableState(
    [int]$WaitMs = 15000,
    [scriptblock]$Condition = $null,
    [string]$Description = 'semantic state'
) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    $signature = $null
    $stableSince = $null
    $validSnapshotCount = 0
    $attempts = 0
    $lastState = $null
    $lastResponse = 'no diagnostic request completed'
    do {
        $remainingMs = Get-RemainingDiagnosticMessageTimeoutMs $deadline
        if ($remainingMs -le 0) { break }
        $attempts++
        try {
            $next = Read-Diagnostic $remainingMs
            $lastState = $next
            $validSnapshotCount++
        } catch [TimeoutException] {
            $lastResponse = $_.Exception.Message
            $signature = $null
            $stableSince = $null
            if ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
            continue
        }
        if ($null -ne $Condition -and -not (& $Condition $next)) {
            $signature = $null
            $stableSince = $null
            $lastResponse = 'fresh valid snapshot did not satisfy the stability condition'
        } else {
            $nextSignature = ConvertTo-Json -InputObject @(
                [long]$next.revision, [string]$next.source,
                [int]$next.selection.anchor, [int]$next.selection.head,
                [bool]$next.dirty, [int]$next.undo_count, [int]$next.redo_count,
                [bool]$next.ime.active, [string]$next.ime.preedit,
                [string]$next.parent_root.editor_rect
            ) -Compress
            if ($nextSignature -ceq $signature) {
                if (([DateTime]::UtcNow - $stableSince).TotalMilliseconds -ge 250) { return $next }
            } else {
                $signature = $nextSignature
                $stableSince = [DateTime]::UtcNow
            }
            $lastResponse = 'fresh valid snapshot stable for {0:N0} ms' -f ([DateTime]::UtcNow - $stableSince).TotalMilliseconds
        }
        if ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    } while ([DateTime]::UtcNow -lt $deadline)
    $kind = if ($validSnapshotCount -eq 0) { 'OBSERVER_FAILURE' } else { 'ENVIRONMENT_BLOCKED' }
    $exception = [TimeoutException]::new($kind + ': ' + $Description + ' did not become stable within ' + $WaitMs +
        ' ms (' + $attempts + ' attempts; last observer response: ' + $lastResponse + ').')
    $exception.Data['LastDiagnosticState'] = $lastState
    $exception.Data['ValidSnapshotCount'] = $validSnapshotCount
    throw $exception
}

function Save-EditorRecoveryEvidence([string]$CaseId, [string]$Phase) {
    Assert-OwnedWindow $script:WindowHandle
    $state = Wait-EditorRecoveryStableState
    $directory = Join-Path $script:UiRunDirectory ('cases\' + $CaseId)
    [void][IO.Directory]::CreateDirectory($directory)
    $script:EditorRecoveryEvidenceOrdinal++
    $stem = '{0:D3}-{1}' -f $script:EditorRecoveryEvidenceOrdinal, ($Phase -replace '[^A-Za-z0-9]+','-').Trim('-')
    $diagnostic = Join-Path $directory ($stem + '.diagnostic.json')
    $sourcePath = Join-Path $directory ($stem + '.source.md')
    $screen = Join-Path $directory ($stem + '.png')
    Copy-Item -LiteralPath $script:DiagnosticPath -Destination $diagnostic -Force
    $sourceBytes = [Text.UTF8Encoding]::new($false).GetBytes([string]$state.source)
    [IO.File]::WriteAllBytes($sourcePath, $sourceBytes)
    Capture-UiOwnedWindow $script:WindowHandle $screen
    if ([MdliteRustAcceptanceNative]::GetForegroundWindow() -ne $script:WindowHandle) {
        throw 'ENVIRONMENT_BLOCKED: owned Editor was not foreground for visible capture.'
    }
    $bounds = [int[]][MdliteRustAcceptanceNative]::WindowRect($script:WindowHandle)
    $client = [int[]][MdliteRustAcceptanceNative]::ClientRect($script:WindowHandle)
    $entry = [pscustomobject]@{
        case_id = $CaseId; phase = $Phase; captured_utc = [DateTime]::UtcNow.ToString('o')
        pid = [int]$script:ProcessId; hwnd = '0x{0:X}' -f $script:WindowHandle.ToInt64()
        executable_path = [string]$script:Report.executable_path; executable_sha256 = [string]$script:Report.executable_sha256_before
        git_head = [string]$script:Report.git_head; harness_sha256 = [string]$script:Report.harness_sha256
        dpi = [int][MdliteRustAcceptanceNative]::GetDpiForWindow($script:WindowHandle)
        window_bounds = $bounds; client_size = @(($client[2] - $client[0]), ($client[3] - $client[1]))
        diagnostic_path = $diagnostic.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
        source_snapshot_path = $sourcePath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
        source_utf8_bytes = $sourceBytes.Length; source_sha256 = Get-EditorRecoveryBytesSha256 $sourceBytes
        screenshot_path = $screen.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
        screenshot_sha256 = (Get-FileHash -LiteralPath $screen -Algorithm SHA256).Hash.ToLowerInvariant()
        revision = [int]$state.revision; selection = $state.selection; dirty = [bool]$state.dirty
        undo_count = [int]$state.undo_count; redo_count = [int]$state.redo_count
        raw = [bool]$state.raw; tables = [int]$state.tables; ime = $state.ime
        active_input_profile = Get-EditorRecoveryProfileIdentity $state.active_input_profile
    }
    $script:Report.evidence += $entry
    return $entry
}

function Set-EditorRecoveryCase([string]$Id, [string]$Status, $Details) {
    $case = @($script:Report.editor_recovery_cases | Where-Object { $_.id -ceq $Id }) | Select-Object -First 1
    if ($null -eq $case) { throw "Unknown EditorRecovery case '$Id'." }
    $case.status = $Status; $case.details = $Details
    if ($Status -eq 'PASS') { $case.reason = $null }
    elseif ($Details -is [System.Collections.IDictionary] -and $Details.Contains('reason')) { $case.reason = [string]$Details.reason }
    $case.evidence = @($script:Report.evidence | Where-Object { $_.case_id -ceq $Id })
}

function Assert-EditorRecoveryDocument([string]$Source, [int]$Head, [bool]$Dirty, [string]$Name) {
    $state = $null
    try {
        $state = Wait-Document $Source $Head $Head $Dirty
    } catch {
        if ($_.Exception -isnot [TimeoutException]) { throw }
        $last = $_.Exception.Data['LastDiagnosticState']
        if ($null -eq $last) { throw }
        $actualHash = Get-UiTextSha256 ([string]$last.source)
        $matchesExpected = $last.source -ceq $Source -and
            [int]$last.selection.anchor -eq $Head -and
            [int]$last.selection.head -eq $Head -and
            [bool]$last.dirty -eq $Dirty
        if ($matchesExpected) { throw }
        throw ('PRODUCT_ASSERTION: {0}; expected_source_sha256={1}; actual_source_sha256={2}; expected_head={3}; actual_selection={4}..{5}; expected_dirty={6}; actual_dirty={7}.' -f
            $Name,(Get-UiTextSha256 $Source),$actualHash,$Head,$last.selection.anchor,$last.selection.head,$Dirty,$last.dirty)
    }
    Add-Observation $Name $state
    return $state
}

function Assert-EditorRecoverySelection(
    [string]$ExpectedSource,
    [long]$DocumentId,
    [int]$Anchor,
    [int]$Head,
    [bool]$Dirty,
    [int]$UndoCount,
    [int]$RedoCount,
    [string]$Description,
    [object]$ExpectedRaw = $null
) {
    $state = $null
    try {
        $state = Wait-Diagnostic {
            param($value)
            [long]$value.document_id -eq $DocumentId -and
                $value.source -ceq $ExpectedSource -and
                [int]$value.selection.anchor -eq $Anchor -and
                [int]$value.selection.head -eq $Head -and
                [bool]$value.dirty -eq $Dirty -and
                [int]$value.undo_count -eq $UndoCount -and
                [int]$value.redo_count -eq $RedoCount -and
                ($null -eq $ExpectedRaw -or ($null -ne $value.raw -and [bool]$value.raw -eq [bool]$ExpectedRaw))
        } $Description
    } catch {
        if ($_.Exception -isnot [TimeoutException]) { throw }
        $last = $_.Exception.Data['LastDiagnosticState']
        if ($null -eq $last) { throw }
        $matchesExpected = [long]$last.document_id -eq $DocumentId -and
            $last.source -ceq $ExpectedSource -and
            [int]$last.selection.anchor -eq $Anchor -and
            [int]$last.selection.head -eq $Head -and
            [bool]$last.dirty -eq $Dirty -and
            [int]$last.undo_count -eq $UndoCount -and
            [int]$last.redo_count -eq $RedoCount -and
            ($null -eq $ExpectedRaw -or ($null -ne $last.raw -and [bool]$last.raw -eq [bool]$ExpectedRaw))
        if ($matchesExpected) { throw }
        throw ('PRODUCT_ASSERTION: {0}; expected_source_sha256={1}; actual_source_sha256={2}; expected_selection={3}..{4}; actual_selection={5}..{6}; expected_dirty={7}; actual_dirty={8}; expected_history={9}/{10}; actual_history={11}/{12}; expected_raw={13}; actual_raw={14}.' -f
            $Description,(Get-UiTextSha256 $ExpectedSource),(Get-UiTextSha256 ([string]$last.source)),
            $Anchor,$Head,$last.selection.anchor,$last.selection.head,$Dirty,$last.dirty,
            $UndoCount,$RedoCount,$last.undo_count,$last.redo_count,$ExpectedRaw,$last.raw)
    }
    Add-Observation $Description $state
    return $state
}

function Move-EditorRecoveryPointerCaret(
    [int]$PhysicalLineIndex,
    [int]$RightCount,
    [int]$ExpectedByte,
    [long]$DocumentId,
    [int]$UndoCount,
    [int]$RedoCount,
    [string]$Description
) {
    if ($PhysicalLineIndex -lt 0 -or $RightCount -lt 0) {
        throw 'SETUP_FAILURE: pointer-fixture caret navigation indices must be nonnegative.'
    }
    Send-Key $script:InputWindowHandle 0x24 ([uint16[]]@(0x11))
    for ($line = 0; $line -lt $PhysicalLineIndex; $line++) {
        Send-Key $script:InputWindowHandle 0x28 ([uint16[]]@(0x11))
    }
    for ($step = 0; $step -lt $RightCount; $step++) {
        Send-Key $script:InputWindowHandle 0x27
    }
    return Assert-EditorRecoverySelection $script:EditorRecoveryPointerSource $DocumentId `
        $ExpectedByte $ExpectedByte $false $UndoCount $RedoCount $Description
}

function Assert-EditorRecoveryFiniteGeometryNumber($Value, [string]$Description, [switch]$Integer) {
    if ($Value -isnot [ValueType] -or [Type]::GetTypeCode($Value.GetType()).ToString() -notin
        @('Byte','SByte','Int16','UInt16','Int32','UInt32','Int64','UInt64','Single','Double','Decimal')) {
        throw ('OBSERVER_FAILURE: {0} is missing or is not a numeric diagnostic field.' -f $Description)
    }
    $number=[double]$Value
    if ([double]::IsNaN($number) -or [double]::IsInfinity($number) -or
        ($Integer -and $number -ne [Math]::Truncate($number))) {
        throw ('OBSERVER_FAILURE: {0} is not a finite diagnostic {1}.' -f $Description,$(if($Integer){'integer'}else{'number'}))
    }
}

function Assert-EditorRecoveryViewGeometryMetadata($State, [switch]$RequireTables) {
    if ($State -isnot [PSCustomObject]) { throw 'OBSERVER_FAILURE: editor view geometry snapshot is missing or malformed.' }
    $collections=@('views')
    if ($RequireTables) { $collections+='table_geometry' }
    foreach ($name in $collections) {
        $property=$State.PSObject.Properties[$name]
        if ($null -eq $property -or $property.Value -isnot [Collections.IList] -or $property.Value -is [string]) {
            throw ('OBSERVER_FAILURE: {0} diagnostic collection is missing or malformed.' -f $name)
        }
        foreach ($item in $property.Value) {
            if ($item -isnot [PSCustomObject]) { throw ('OBSERVER_FAILURE: {0} diagnostic item is malformed.' -f $name) }
            $fields=@('source_start','source_end','y','width','height')
            if ($name -ceq 'views') { $fields+='x' }
            foreach ($field in $fields) {
                $member=$item.PSObject.Properties[$field]
                if ($null -eq $member) { throw ('OBSERVER_FAILURE: {0}.{1} diagnostic member is missing.' -f $name,$field) }
                Assert-EditorRecoveryFiniteGeometryNumber $member.Value ($name+'.'+$field) -Integer:($field -in @('source_start','source_end'))
            }
            if ($name -ceq 'views') {
                $table=$item.PSObject.Properties['table']; $map=$item.PSObject.Properties['map']
                if ($null -eq $table -or $null -eq $map -or $map.Value -isnot [Collections.IList] -or $map.Value -is [string]) {
                    throw 'OBSERVER_FAILURE: view table identity or source-map collection is missing or malformed.'
                }
                if ($null -ne $table.Value) { Assert-EditorRecoveryFiniteGeometryNumber $table.Value 'view table identity' -Integer }
                foreach ($point in $map.Value) { Assert-EditorRecoveryFiniteGeometryNumber $point 'view source-map byte' -Integer }
            }
        }
    }
}

function Get-EditorRecoveryCaretDipPoint($State, [string]$Description) {
    $nativeCaretProperty = $null
    if ($null -ne $State) { $nativeCaretProperty = $State.PSObject.Properties['native_caret'] }
    if ($null -eq $nativeCaretProperty -or $nativeCaretProperty.Value -isnot [PSCustomObject]) {
        throw ('OBSERVER_FAILURE: {0} has no native caret position and size.' -f $Description)
    }
    $nativeCaret = $nativeCaretProperty.Value
    $positionProperty = $nativeCaret.PSObject.Properties['position']
    $sizeProperty = $nativeCaret.PSObject.Properties['size']
    if ($null -eq $positionProperty -or $null -eq $sizeProperty) {
        throw ('OBSERVER_FAILURE: {0} has no native caret position and size.' -f $Description)
    }
    $position = $positionProperty.Value
    $size = $sizeProperty.Value
    if ($position -isnot [System.Collections.IList] -or $position -is [string] -or $position.Count -ne 2 -or
        $size -isnot [System.Collections.IList] -or $size -is [string] -or $size.Count -ne 2) {
        throw ('OBSERVER_FAILURE: {0} has no native caret position and size.' -f $Description)
    }
    foreach ($number in @($position[0],$position[1],$size[0],$size[1])) {
        Assert-EditorRecoveryFiniteGeometryNumber $number ($Description+' native caret pixels') -Integer
    }
    $parentProperty=$State.PSObject.Properties['parent_root']
    if ($null -eq $parentProperty -or $parentProperty.Value -isnot [PSCustomObject]) { throw 'OBSERVER_FAILURE: caret parent geometry is missing or malformed.' }
    $metadata=$parentProperty.Value
    $dpiProperty=$metadata.PSObject.Properties['dpi']; $bodyProperty=$metadata.PSObject.Properties['active_editor_rect']
    if ($null -eq $dpiProperty -or $null -eq $bodyProperty -or $bodyProperty.Value -isnot [PSCustomObject]) { throw 'OBSERVER_FAILURE: caret DPI or editor viewport geometry is missing or malformed.' }
    Assert-EditorRecoveryFiniteGeometryNumber $dpiProperty.Value ($Description+' Main DPI')
    $dpi=[double]$dpiProperty.Value; $body=$bodyProperty.Value
    foreach ($field in @('x','y','width','height')) {
        $member=$body.PSObject.Properties[$field]
        if ($null -eq $member) { throw ('OBSERVER_FAILURE: caret viewport {0} is missing.' -f $field) }
        Assert-EditorRecoveryFiniteGeometryNumber $member.Value ($Description+' viewport '+$field)
    }
    if ($dpi -le 0 -or [double]$body.width -le 0 -or [double]$body.height -le 0 -or $size[0] -le 0 -or $size[1] -le 0) {
        throw 'PRODUCT_ASSERTION: present caret/viewport geometry or Main DPI is nonpositive.'
    }
    [void](Get-UiParentRoot $State)
    $clientX = [int]$position[0]
    $clientY = [int]$position[1] + [int][Math]::Floor([double]$size[1] / 2.0)
    $dipX = [double]$clientX / $dpi
    $dipY = [double]$clientY / $dpi
    if ($dipX -lt [double]$body.x -or $dipX -gt ([double]$body.x + [double]$body.width) -or
        $dipY -lt [double]$body.y -or $dipY -gt ([double]$body.y + [double]$body.height)) {
        throw ('ENVIRONMENT_BLOCKED: {0} native caret lies outside the owned Main editor viewport.' -f $Description)
    }
    return [pscustomobject]@{
        x=$dipX; y=$dipY; dpi=$dpi
        native_position_px=@($clientX,[int]$position[1]); native_size_px=@([int]$size[0],[int]$size[1])
    }
}

function Invoke-EditorRecoveryIndentationLine(
    [string]$FixtureSource,
    [long]$DocumentId,
    [int]$LineNumber,
    [string]$ExpectedLine,
    [int]$UndoCount,
    [int]$RedoCount
) {
    if ($FixtureSource.Contains("`r")) { throw 'SETUP_FAILURE: E07 fixture-line oracle expects LF source.' }
    $lines=[regex]::Split($FixtureSource,'\n')
    if ($LineNumber -lt 1 -or $LineNumber -gt $lines.Length) {
        throw 'SETUP_FAILURE: E07 physical line number is outside the fixed fixture.'
    }
    $lineText=[string]$lines[$LineNumber-1]
    if ($lineText -cne $ExpectedLine) {
        throw ('SETUP_FAILURE: E07 line {0} expected {1}, got {2}.' -f $LineNumber,$ExpectedLine,$lineText)
    }
    $lineStartChar=0
    for ($line=0; $line -lt ($LineNumber-1); $line++) {
        $lineStartChar += $lines[$line].Length + 1
    }
    $startByte=[Text.Encoding]::UTF8.GetByteCount($FixtureSource.Substring(0,$lineStartChar))
    $endByte=$startByte+[Text.Encoding]::UTF8.GetByteCount($lineText)

    Send-Key $script:InputWindowHandle 0x24 ([uint16[]]@(0x11))
    for ($line=0; $line -lt ($LineNumber-1); $line++) {
        Send-Key $script:InputWindowHandle 0x28 ([uint16[]]@(0x11))
    }
    [void](Assert-EditorRecoverySelection $FixtureSource $DocumentId $startByte $startByte $false $UndoCount $RedoCount ('E07 line {0} start' -f $LineNumber))
    Send-Key $script:InputWindowHandle 0x24
    Send-Key $script:InputWindowHandle 0x23 ([uint16[]]@(0x10))
    $selected=Assert-EditorRecoverySelection $FixtureSource $DocumentId $startByte $endByte $false $UndoCount $RedoCount ('E07 select line {0}' -f $LineNumber)

    $indentedSource=$FixtureSource.Insert($lineStartChar,'    ')
    Send-Key $script:InputWindowHandle 0x09
    $indented=Assert-EditorRecoverySelection $indentedSource $DocumentId `
        ($startByte+4) ($endByte+4) $true ($UndoCount+1) 0 ('E07 Tab indents line {0}' -f $LineNumber)
    [void](Save-EditorRecoveryEvidence 'E07' ('line-{0}-tab' -f $LineNumber))
    Send-Key $script:InputWindowHandle 0x09 ([uint16[]]@(0x10))
    $unindented=Assert-EditorRecoverySelection $FixtureSource $DocumentId `
        $startByte $endByte $true ($UndoCount+2) 0 ('E07 Shift+Tab removes line indentation but remains dirty {0}' -f $LineNumber)
    [void](Save-EditorRecoveryEvidence 'E07' ('line-{0}-shift-tab' -f $LineNumber))
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    $undoShiftTab=Assert-EditorRecoverySelection $indentedSource $DocumentId `
        ($startByte+4) ($endByte+4) $true ($UndoCount+1) 1 ('E07 Undo Shift+Tab restores indentation {0}' -f $LineNumber)
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    $restored=Assert-EditorRecoverySelection $FixtureSource $DocumentId `
        $startByte $endByte $false $UndoCount 2 ('E07 Undo Tab restores saved checkpoint {0}' -f $LineNumber)
    $revisionStates=@($selected,$indented,$unindented,$undoShiftTab,$restored)
    for ($revisionIndex=1; $revisionIndex -lt $revisionStates.Length; $revisionIndex++) {
        if ([long]$revisionStates[$revisionIndex].revision -le [long]$revisionStates[$revisionIndex-1].revision) {
            throw ('PRODUCT_ASSERTION: E07 line {0} edit/Undo revision did not advance monotonically.' -f $LineNumber)
        }
    }
    [void](Save-EditorRecoveryEvidence 'E07' ('line-{0}-undo-restored-saved-state' -f $LineNumber))
    return [pscustomobject]@{
        line=$LineNumber; expected_line=$lineText; selection_before=@($startByte,$endByte)
        source_after_tab_sha256=Get-UiTextSha256 ([string]$indented.source)
        source_after_shift_tab_sha256=Get-UiTextSha256 ([string]$unindented.source)
        source_after_undo_shift_tab_sha256=Get-UiTextSha256 ([string]$undoShiftTab.source)
        source_after_undo_tab_sha256=Get-UiTextSha256 ([string]$restored.source)
        dirty_after_shift_tab=[bool]$unindented.dirty; dirty_after_undo_tab=[bool]$restored.dirty
        selection_after_undo_shift_tab=@([int]$undoShiftTab.selection.anchor,[int]$undoShiftTab.selection.head)
        selection_after_undo_tab=@([int]$restored.selection.anchor,[int]$restored.selection.head)
        revision_before=[long]$selected.revision; revision_after_undo_tab=[long]$restored.revision
        undo_count_after_tab=[int]$indented.undo_count; undo_count_after_shift_tab=[int]$unindented.undo_count
        undo_count_after=[int]$restored.undo_count; redo_count_after=[int]$restored.redo_count
    }
}

function Test-EditorRecoveryProfileIdentity($Expected, $Actual) {
    if ($null -eq $Expected -or $null -eq $Actual) { return $false }
    foreach ($name in @('status','profile_type','language_id','hkl')) {
        $left = $Expected.PSObject.Properties[$name]
        $right = $Actual.PSObject.Properties[$name]
        if ($null -eq $left -or $null -eq $right -or [string]$left.Value -cne [string]$right.Value) { return $false }
    }
    foreach ($name in @('clsid','profile_guid')) {
        $left = $Expected.PSObject.Properties[$name]
        $right = $Actual.PSObject.Properties[$name]
        if ($null -eq $left -or $null -eq $right) { return $false }
        $leftGuid = [guid]::Empty
        $rightGuid = [guid]::Empty
        if (-not [guid]::TryParse([string]$left.Value, [ref]$leftGuid) -or
            -not [guid]::TryParse([string]$right.Value, [ref]$rightGuid) -or
            $leftGuid -ne $rightGuid) { return $false }
    }
    return $true
}

function Reset-EditorRecoveryAsciiPin($State, [string]$Reason) {
    $layout = [MdliteRustAcceptanceNative]::OwnedWindowKeyboardLayout($script:InputWindowHandle, $script:ProcessId)
    if ($layout -eq [IntPtr]::Zero) { throw 'ENVIRONMENT_BLOCKED: owned Editor keyboard layout could not be read after profile activation.' }
    $layoutText = '0x{0:X}' -f $layout.ToInt64()
    $caps = [MdliteRustAcceptanceNative]::CapsLockIsOn()
    $profile = Get-EditorRecoveryProfileIdentity $State.active_input_profile
    $previousProfile = $script:Report.ascii_input_profile_current
    $previousLayout = [string]$script:Report.ascii_keyboard_layout
    $previousCaps = $script:Report.ascii_caps_lock_on
    $profileChanged = $null -ne $previousProfile -and
        -not (Test-EditorRecoveryProfileIdentity $previousProfile $profile)
    $layoutChanged = $null -ne $previousLayout -and $previousLayout -cne $layoutText
    if ($profileChanged -or $layoutChanged) {
        $script:Report.ascii_keyboard_layout_transitions += [pscustomobject]@{
            reason = $Reason
            previous_profile = $previousProfile
            current_profile = $profile
            previous_layout = $previousLayout
            previous_caps_lock = $previousCaps
            current_layout = $layoutText
            current_caps_lock = $caps
            authorized_profile_change = $true
            observed_utc = [DateTime]::UtcNow.ToString('o')
        }
        if ($layoutChanged) { $script:Report.ascii_keyboard_layout = $null }
    }
    $script:Report.ascii_input_profile_current = $profile
}

function Request-EditorRecoveryProfile([string]$CaseId, [string]$Pattern, $ExpectedIdentity = $null) {
    $id = [guid]::NewGuid().ToString('N')
    $requestPath = Join-Path $script:UiRunDirectory ('ime-profile-' + $CaseId + '.json')
    $before = Read-Diagnostic
    $requested = if ($ExpectedIdentity) { 'initial' } else { $Pattern }
    $request = [ordered]@{
        request_id = $id; case_id = $CaseId; requested_profile = $requested
        description_pattern = $Pattern; expected_identity = $ExpectedIdentity
        initial_identity = Get-EditorRecoveryProfileIdentity $before.active_input_profile
        target_pid = [int]$script:ProcessId; target_hwnd = '0x{0:X}' -f $script:WindowHandle.ToInt64()
        activation_scope = 'TSF FORSESSION; no registry/default-profile change'
        status = 'PENDING'
    }
    [IO.File]::WriteAllText($requestPath, ($request | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $relative = $requestPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
    $requestRecord = [pscustomobject]@{
        request_id=$id; case_id=$CaseId; requested_profile=$requested; request_path=$relative
        activation_scope=$request.activation_scope; other_app_profiles='NOT_VERIFIED'; status='PENDING'
        initial_profile=Get-EditorRecoveryProfileIdentity $before.active_input_profile
        available_profiles=@(); selected_profile=$null; observed_profile=$null; hresult=$null
        api_error=$null; fallback_used=$false
    }
    $script:Report.profile_requests += $requestRecord
    Set-OwnedForeground -Window $script:WindowHandle -WaitMs 15000

    $apiError = $null
    $selected = $null
    if ($ExpectedIdentity) {
        if ([string]$ExpectedIdentity.profile_type -notin @('text_service','keyboard_layout')) {
            $requestRecord.status = 'BLOCKED_INITIAL_PROFILE_TYPE_UNKNOWN'
            [IO.File]::WriteAllText($requestPath, ($requestRecord | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
            return $null
        }
        $selected = [pscustomobject]@{
            profile_type_name = [string]$ExpectedIdentity.profile_type
            language_id_value = [Convert]::ToUInt16(([string]$ExpectedIdentity.language_id).Replace('0x',''), 16)
            language_id = [string]$ExpectedIdentity.language_id
            clsid = [string]$ExpectedIdentity.clsid
            profile_guid = [string]$ExpectedIdentity.profile_guid
            hkl = [string]$ExpectedIdentity.hkl
        }
    } else {
        try {
            if ([Threading.Thread]::CurrentThread.ApartmentState -ne [Threading.ApartmentState]::STA) {
                throw 'The harness thread is not STA; TSF COM activation is unavailable.'
            }
            $available = @([MdliteRustAcceptanceNative]::EnumerateJapaneseTextProfiles())
            $requestRecord.available_profiles = @($available | ForEach-Object {
                [pscustomobject]@{
                    description=$_.description; profile_guid=$_.profile_guid; clsid=$_.clsid
                    enabled=[bool]$_.enabled; active=[bool]$_.active; hkl=$_.hkl
                }
            })
            $matches = @($available | Where-Object {
                [bool]$_.enabled -and [string]$_.description -match $Pattern
            })
            if ($matches.Count -eq 0) {
                $requestRecord.status = 'BLOCKED_PROFILE_NOT_INSTALLED_OR_ENABLED'
                [IO.File]::WriteAllText($requestPath, ($requestRecord | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
                return $null
            }
            $activeMatches = @($matches | Where-Object { [bool]$_.active })
            if ($activeMatches.Count -eq 1) { $selected = $activeMatches[0] }
            elseif ($matches.Count -eq 1) { $selected = $matches[0] }
            else {
                $requestRecord.status = 'BLOCKED_AMBIGUOUS_INSTALLED_PROFILES'
                [IO.File]::WriteAllText($requestPath, ($requestRecord | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
                return $null
            }
        } catch {
            $apiError = $_.Exception.Message
        }
    }

    if ($null -ne $selected) {
        $profileType = if ([string]$selected.profile_type_name -eq 'text_service') { [uint32]1 } else { [uint32]2 }
        try {
            $hresult = [MdliteRustAcceptanceNative]::ActivateInputProfile(
                $profileType, [uint16]$selected.language_id_value, [string]$selected.clsid,
                [string]$selected.profile_guid, [string]$selected.hkl)
        } catch {
            $apiError = $_.Exception.Message
            $hresult = $null
        }
        if ($null -eq $hresult) { $selected = $null }
    }
    if ($null -ne $selected) {
        $hresultText = '0x{0:X8}' -f [int]$hresult
        $requestRecord.hresult = $hresultText
        $requestRecord.selected_profile = $selected
        if ($hresult -eq 0) {
            $expected = if ($ExpectedIdentity) { $ExpectedIdentity } else {
                [pscustomobject]@{
                    status='active'; profile_type=[string]$selected.profile_type_name
                    language_id=[string]$selected.language_id; clsid=[string]$selected.clsid
                    profile_guid=[string]$selected.profile_guid; hkl=[string]$selected.hkl
                }
            }
            $deadline = [DateTime]::UtcNow.AddSeconds(15)
            $observed = $null
            while ([DateTime]::UtcNow -lt $deadline) {
                $remainingMs = Get-RemainingDiagnosticMessageTimeoutMs $deadline
                if ($remainingMs -le 0) { break }
                try {
                    $candidate = Read-Diagnostic $remainingMs
                    $matchesReadback = Test-EditorRecoveryProfileIdentity $expected $candidate.active_input_profile
                    if ($matchesReadback) { $observed = $candidate; break }
                } catch [TimeoutException] {}
                Start-Sleep -Milliseconds 150
            }
            if ($observed) {
                $requestRecord.status = 'PASS_TSF_FORSESSION_READBACK'
                $requestRecord.observed_profile = Get-EditorRecoveryProfileIdentity $observed.active_input_profile
                $script:Report.profile_requests[-1] = $requestRecord
                Reset-EditorRecoveryAsciiPin $observed $CaseId
                [IO.File]::WriteAllText($requestPath, ($requestRecord | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
                return $observed
            }
            $requestRecord.status = 'BLOCKED_OWNER_PROFILE_READBACK_MISMATCH'
            $requestRecord.api_error = 'TSF returned S_OK but the owned Editor diagnostic did not match the requested profile.'
            [IO.File]::WriteAllText($requestPath, ($requestRecord | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
            return $null
        }
        $requestRecord.status = if ($hresult -eq 1) { 'BLOCKED_PROFILE_NOT_ENABLED' } else { 'BLOCKED_TSF_ACTIVATION_FAILED' }
        $requestRecord.api_error = 'ITfInputProcessorProfileMgr::ActivateProfile returned {0}.' -f $hresultText
        [IO.File]::WriteAllText($requestPath, ($requestRecord | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
        return $null
    }

    $requestRecord.status = 'FALLBACK_NORMAL_SELECTOR'
    $requestRecord.fallback_used = $true
    $requestRecord.api_error = $apiError
    [IO.File]::WriteAllText($requestPath, ($requestRecord | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    Write-Host ('IME_PROFILE_FALLBACK_REQUIRED case={0} profile={1} pid={2} hwnd=0x{3:X} request={4}' -f
        $CaseId,$requested,$script:ProcessId,$script:WindowHandle.ToInt64(),$relative)
    $fallbackDeadline = [DateTime]::UtcNow.AddSeconds(45)
    while ([DateTime]::UtcNow -lt $fallbackDeadline) {
        $remainingMs = Get-RemainingDiagnosticMessageTimeoutMs $fallbackDeadline
        if ($remainingMs -le 0) { break }
        try {
            $candidate = Read-Diagnostic $remainingMs
            $actual = $candidate.active_input_profile
            $matchesFallback = if ($ExpectedIdentity) {
                Test-EditorRecoveryProfileIdentity $ExpectedIdentity $actual
            } else {
                $null -ne $actual -and [string]$actual.status -ceq 'active' -and
                [string]$actual.profile_type -ceq 'text_service' -and
                [string]$actual.language_id -ceq '0x0411' -and [string]$actual.description -match $Pattern
            }
            if ($matchesFallback) {
                $requestRecord.status = 'PASS_NORMAL_SELECTOR_READBACK'
                $requestRecord.observed_profile = Get-EditorRecoveryProfileIdentity $actual
                $script:Report.profile_requests[-1] = $requestRecord
                Reset-EditorRecoveryAsciiPin $candidate $CaseId
                [IO.File]::WriteAllText($requestPath, ($requestRecord | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
                return $candidate
            }
        } catch [TimeoutException] {}
        Start-Sleep -Milliseconds 250
    }
    $requestRecord.status = 'BLOCKED_PROFILE_REQUEST_TIMEOUT'
    $script:Report.profile_requests[-1] = $requestRecord
    [IO.File]::WriteAllText($requestPath, ($requestRecord | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    return $null
}

function Invoke-EditorRecoveryImeCase([string]$CaseId, [string]$Pattern) {
    $script:Report.ime_status='PENDING'
    $script:Report.ime_committed_suffix=$null
    $script:Report.ime_composition_preedit=$null
    $script:Report.ime_conversion_candidate=$null
    $script:Report.ime_scripted_reading=$null
    $script:Report.ime_expected_reading=$null
    $script:Report.ime_space_conversion=$null
    $script:Report.ime_candidate_movement=$null
    $script:Report.ime_composition_invariants=@()
    $script:Report.ime_escape_stages=@()
    $state = Switch-ToSource $script:EditorRecoveryExpectedSource
    $source = [string]$state.source
    if ($source -cne $script:EditorRecoveryExpectedSource) {
        throw ('PRODUCT_ASSERTION: {0} selected a fixture with unexpected source.' -f $CaseId)
    }
    $end = [Text.Encoding]::UTF8.GetByteCount($source)
    Send-Key $script:InputWindowHandle 0x23 ([uint16[]]@(0x11))
    [void](Assert-EditorRecoveryDocument $source $end ([bool]$state.dirty) ($CaseId + ' source EOF'))
    $profile = Request-EditorRecoveryProfile $CaseId $Pattern
    if ($null -eq $profile) {
        Set-EditorRecoveryCase $CaseId 'BLOCKED' @{ requested_profile=$Pattern; reason='No matching active Japanese text service within 45 seconds.' }
        return $null
    }
    $script:EditorRecoveryCurrentCaseId = $CaseId
    $script:ObservationCallback = {
        param($name,$state)
        if ($name -match 'composition|candidate|cancellation|commit') {
            $phase = 'ime-' + (($name -replace '[^A-Za-z0-9]+','-').Trim('-'))
            [void](Save-EditorRecoveryEvidence $script:EditorRecoveryCurrentCaseId $phase)
        }
    }
    try { $state = Invoke-ConfiguredImeAcceptance $source $end }
    finally { $script:ObservationCallback = $null; $script:EditorRecoveryCurrentCaseId = $null }
    if ([string]$script:Report.ime_status -like 'BLOCKED:*') {
        Set-EditorRecoveryCase $CaseId 'BLOCKED' @{
            profile=Get-EditorRecoveryProfileIdentity $profile.active_input_profile
            composition_preedit=$script:Report.ime_composition_preedit
            scripted_reading=$script:Report.ime_scripted_reading
            expected_reading=$script:Report.ime_expected_reading
            space_conversion=$script:Report.ime_space_conversion
            candidate_movement=$script:Report.ime_candidate_movement
            escape_stages=@($script:Report.ime_escape_stages)
            composition_invariants=@($script:Report.ime_composition_invariants)
            converted_candidate=$script:Report.ime_conversion_candidate
            reason=[string]$script:Report.ime_status
        }
        return $null
    }
    if ([string]$script:Report.ime_status -notlike 'PASS:*' -and
        [string]$script:Report.ime_status -notlike 'PARTIAL:*') {
        throw ('SETUP_FAILURE: {0} IME helper returned an unclassified status: {1}' -f $CaseId,$script:Report.ime_status)
    }
    $suffix = [string]$script:Report.ime_committed_suffix
    if ([string]::IsNullOrEmpty($suffix)) { throw ('PRODUCT_ASSERTION: {0} committed no source suffix.' -f $CaseId) }
    $candidate = [string]$script:Report.ime_conversion_candidate
    if (-not (Test-EditorRecoveryJapaneseText $candidate)) {
        throw ('PRODUCT_ASSERTION: {0} passed without a Japanese converted candidate from the real IME preedit.' -f $CaseId)
    }
    if ($suffix -cne $candidate) {
        throw ('PRODUCT_ASSERTION: {0} committed suffix did not match the captured converted IME candidate.' -f $CaseId)
    }
    $expected = $source + $suffix
    $state = Assert-EditorRecoveryDocument $expected ([Text.Encoding]::UTF8.GetByteCount($expected)) $true ($CaseId + ' exact committed suffix')
    $script:EditorRecoveryExpectedSource = $expected
    [void](Save-EditorRecoveryEvidence $CaseId 'ime-final')
    $coreStatus = if ([string]$script:Report.ime_status -like 'PASS:*') { 'PASS' } else { 'PARTIAL' }
    $remaining = @('post-commit reconversion and phrase replacement branch','candidate UI screenshot review')
    if ([string]$script:Report.ime_candidate_movement.status -cne 'PASS') {
        $remaining += 'candidate movement could not be proven from a distinct stable diagnostic preedit'
    }
    Set-EditorRecoveryCase $CaseId 'NOT_RUN' @{
        profile=Get-EditorRecoveryProfileIdentity $profile.active_input_profile
        core_sequence_status=$coreStatus
        cancellation_source_unchanged=$true
        scripted_reading=$script:Report.ime_scripted_reading
        expected_reading=$script:Report.ime_expected_reading
        composition_preedit=[string]$script:Report.ime_composition_preedit
        space_conversion=$script:Report.ime_space_conversion
        candidate_movement=$script:Report.ime_candidate_movement
        escape_stages=@($script:Report.ime_escape_stages)
        candidate_ui_screenshot_review='NOT_RUN'
        composition_invariants=@($script:Report.ime_composition_invariants)
        converted_candidate=$candidate; committed_suffix=$suffix
        committed_source_sha256=Get-UiTextSha256 $expected
        not_run=$remaining
        reason='Core scripted IME evidence is recorded; candidate UI screenshot review and post-commit reconversion remain NOT_RUN.'
    }
    return $state
}

# Private definitions only. Integration executes these in the existing owned
# Workspace process after each successful core profile case, before E05.

function Assert-EditorRecoveryExtraAffinity($State,$Baseline,[string]$Phase) {
    foreach ($value in @($State,$Baseline)) {
        if ($null -eq $value -or $null -eq $value.PSObject.Properties['selection_affinity'] -or
            $value.selection_affinity -isnot [string] -or $value.selection_affinity -cnotin @('Before','After')) {
            throw ('OBSERVER_FAILURE: '+$Phase+' lacks a valid exposed selection_affinity.')
        }
    }
    if ($State.selection_affinity -cne $Baseline.selection_affinity) { throw ('PRODUCT_ASSERTION: '+$Phase+' changed boundary affinity.') }
}

function Assert-EditorRecoveryExtraInvariant($State,$Baseline,[string]$Phase) {
    Assert-EditorRecoveryExtraAffinity $State $Baseline $Phase
    foreach ($value in @($State,$Baseline)) {
        foreach ($field in @('document_id','tabs')) {
            if ($null -eq $value.PSObject.Properties[$field]) { throw ('OBSERVER_FAILURE: '+$Phase+' lacks '+$field+'.') }
            Assert-EditorRecoveryFiniteGeometryNumber $value.PSObject.Properties[$field].Value ($Phase+' '+$field) -Integer
        }
    }
    if ($null -eq $State.PSObject.Properties['ime'] -or $null -eq $State.ime -or
        $null -eq $State.ime.PSObject.Properties['active'] -or $State.ime.active -isnot [bool] -or
        $null -eq $State.ime.PSObject.Properties['preedit'] -or $State.ime.preedit -isnot [string]) {
        throw ('OBSERVER_FAILURE: '+$Phase+' lacks typed composition metadata.')
    }
    Assert-EditorRecoveryImeInvariant $State $Baseline $Phase
    if ([long]$State.document_id -ne [long]$Baseline.document_id -or [int]$State.tabs -ne [int]$Baseline.tabs) {
        throw ('PRODUCT_ASSERTION: '+$Phase+' changed document/tab ownership.')
    }
}

function Assert-EditorRecoveryAtokReconversionCancelInvariant($State,$Baseline,[string]$Phase) {
    Assert-EditorRecoveryFiniteGeometryNumber $State.revision ($Phase+' state revision') -Integer
    Assert-EditorRecoveryFiniteGeometryNumber $Baseline.revision ($Phase+' baseline revision') -Integer
    if ([long]$State.revision -ne [long]$Baseline.revision) {
        throw ('PRODUCT_ASSERTION: '+$Phase+' changed revision.')
    }
    $selectionPreserved=[int]$State.selection.anchor -eq [int]$Baseline.selection.anchor -and
        [int]$State.selection.head -eq [int]$Baseline.selection.head -and
        $State.selection_affinity -ceq $Baseline.selection_affinity
    $selectionCollapsed=[int]$State.selection.anchor -eq [int]$State.selection.head
    if (-not $selectionPreserved -and -not $selectionCollapsed) {
        throw ('PRODUCT_ASSERTION: '+$Phase+' must preserve the directed selection or collapse it to a valid source position.')
    }
    Assert-SelectionBoundary $State
    $acceptedSelectionBaseline=[pscustomobject]@{
        source=$Baseline.source;selection=$State.selection;selection_affinity=$State.selection_affinity
        dirty=$Baseline.dirty;undo_count=$Baseline.undo_count;redo_count=$Baseline.redo_count
        document_id=$Baseline.document_id;tabs=$Baseline.tabs
    }
    Assert-EditorRecoveryExtraInvariant $State $acceptedSelectionBaseline $Phase
}

function Invoke-EditorRecoveryReconversionProbe($Baseline,[string]$Pattern,$Evidence) {
    # Operational evidence only: custom key assignments can differ from the
    # documented templates. No template receipt/query/settings change is used.
    $routes=if ($Pattern -ceq 'ATOK') {
        @([pscustomobject]@{name='Shift+Convert';modifiers=[uint16[]]@(0x10)},[pscustomobject]@{name='Convert';modifiers=[uint16[]]@()})
    } elseif ($Pattern -ceq 'Microsoft IME') {
        @([pscustomobject]@{name='Convert';modifiers=[uint16[]]@()},[pscustomobject]@{name='Shift+Convert';modifiers=[uint16[]]@(0x10)})
    } else { throw 'SETUP_FAILURE: reconversion probe supports only the two authorized installed profile patterns.' }
    $Evidence['tested_operational_routes']=@();$Evidence['global_key_template']='UNVERIFIED'
    foreach ($route in $routes) {
        $fresh=Read-Diagnostic
        Assert-EditorRecoveryExtraInvariant $fresh $Baseline 'before bounded reconversion route'
        if ([bool]$fresh.ime.active -or -not [string]::IsNullOrEmpty([string]$fresh.ime.preedit)) { throw 'ENVIRONMENT_BLOCKED: reconversion probe already composing; no alternative key was sent.' }
        $script:ImeEscapesSentForComposition=0
        Send-Key $script:InputWindowHandle 0x1C $route.modifiers
        try {
            $composed=Wait-EditorRecoveryStableState -WaitMs 5000 -Description ('documented reconversion '+$route.name) -Condition {
                param($s)
                Assert-EditorRecoveryExtraInvariant $s $Baseline 'documented reconversion'
                [bool]$s.ime.active -and (Test-EditorRecoveryJapaneseText ([string]$s.ime.preedit))
            }
            $Evidence['tested_operational_routes']+=@{route=$route.name;status='OBSERVED_JAPANESE_COMPOSITION';profile=Get-EditorRecoveryProfileIdentity $composed.active_input_profile}
            return [pscustomobject]@{state=$composed;route=$route.name;modifiers=$route.modifiers}
        } catch [TimeoutException] {
            $last=$_.Exception.Data['LastDiagnosticState']
            if ($null -eq $last -or [int]$_.Exception.Data['ValidSnapshotCount'] -le 0) { throw }
            Assert-EditorRecoveryExtraInvariant $last $Baseline 'reconversion last fresh snapshot'
            if ([bool]$last.ime.active -or -not [string]::IsNullOrEmpty([string]$last.ime.preedit)) { throw 'ENVIRONMENT_BLOCKED: documented reconversion produced no proven Japanese composition; no alternative was sent while active.' }
            $Evidence['tested_operational_routes']+=@{route=$route.name;status='NOT_RUN_NO_COMPOSITION_WITHIN_EXISTING_5S';profile=Get-EditorRecoveryProfileIdentity $last.active_input_profile}
        }
    }
    return $null
}

function Select-EditorRecoveryCommittedSuffix(
    [string]$Source,[long]$DocumentId,[string]$Suffix,[bool]$Dirty,
    [int]$Undo,[int]$Redo,[string]$CaseId,[string]$Phase
) {
    if ([string]::IsNullOrEmpty($Suffix) -or -not $Source.EndsWith($Suffix,[StringComparison]::Ordinal) -or $Suffix.Contains("`n") -or $Suffix.Contains("`r")) {
        throw 'SETUP_FAILURE: reconversion target must be the exact captured committed suffix.'
    }
    $state=Switch-ToSource $Source
    if ([long]$state.document_id -ne $DocumentId) { throw 'SETUP_FAILURE: reconversion selected the wrong document.' }
    Assert-EditorRecoveryExtraAffinity $state $state 'reconversion selection metadata'
    [void](Invoke-EditorViewportBodyClick $CaseId ($Phase+' native focus'))
    if (-not [bool]$state.raw) {
        Send-Key $script:InputWindowHandle 0x45 ([uint16[]]@(0x11))
        [void](Wait-Diagnostic {param($s) [long]$s.document_id -eq $DocumentId -and $s.source -ceq $Source -and [bool]$s.raw} ($Phase+' Source selection mode'))
    }
    $end=[Text.Encoding]::UTF8.GetByteCount($Source)
    $start=$end-[Text.Encoding]::UTF8.GetByteCount($Suffix)
    Send-Key $script:InputWindowHandle 0x23 ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $Source $DocumentId $end $end $Dirty $Undo $Redo ($Phase+' exact EOF'))
    $steps=[Globalization.StringInfo]::ParseCombiningCharacters($Suffix).Count
    for ($i=0;$i -lt $steps;$i++) { Send-Key $script:InputWindowHandle 0x25 ([uint16[]]@(0x10)) }
    [void](Assert-EditorRecoverySelection $Source $DocumentId $end $start $Dirty $Undo $Redo ($Phase+' exact reverse committed selection'))
    Send-Key $script:InputWindowHandle 0x45 ([uint16[]]@(0x11))
    $selected=Assert-EditorRecoverySelection $Source $DocumentId $end $start $Dirty $Undo $Redo ($Phase+' reverse selection in Live') $false
    if ([bool]$selected.raw) { throw 'PRODUCT_ASSERTION: reconversion selection did not return to Live.' }
    Assert-EditorRecoveryExtraAffinity $selected $selected 'selected reconversion affinity metadata'
    return $selected
}

function Invoke-EditorRecoveryLiteralImeCommitCycle(
    $Baseline,[string]$ExpectedSource,[int]$ExpectedHead,[string]$CaseId,[string]$Phase
) {
    Assert-EditorRecoveryExtraAffinity $Baseline $Baseline ($Phase+' baseline metadata')
    Send-Key $script:InputWindowHandle 0x0D
    $committed=Wait-EditorRecoveryStableState -WaitMs 10000 -Description ($Phase+' literal native commit') -Condition {
        param($s)
        Assert-EditorRecoveryExtraAffinity $s ([pscustomobject]@{selection_affinity='After'}) ($Phase+' committed affinity')
        (Test-SameInputProfile $s) -and [long]$s.document_id -eq [long]$Baseline.document_id -and
            [int]$s.tabs -eq [int]$Baseline.tabs -and -not [bool]$s.ime.active -and
            [string]::IsNullOrEmpty([string]$s.ime.preedit) -and $s.source -ceq $ExpectedSource -and
            [int]$s.selection.anchor -eq $ExpectedHead -and [int]$s.selection.head -eq $ExpectedHead -and
            [bool]$s.dirty -and [int]$s.undo_count -eq ([int]$Baseline.undo_count+1) -and [int]$s.redo_count -eq 0
    }
    [void](Save-EditorRecoveryEvidence $CaseId ($Phase+' committed'))
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    $undone=Assert-EditorRecoverySelection $Baseline.source $Baseline.document_id $Baseline.selection.anchor $Baseline.selection.head $Baseline.dirty $Baseline.undo_count 1 ($Phase+' one Undo restores original directed selection')
    Assert-EditorRecoveryExtraAffinity $undone $Baseline ($Phase+' one Undo')
    Send-Key $script:InputWindowHandle 0x59 ([uint16[]]@(0x11))
    $redone=Assert-EditorRecoverySelection $ExpectedSource $Baseline.document_id $ExpectedHead $ExpectedHead $true ([int]$Baseline.undo_count+1) 0 ($Phase+' Redo literal commit')
    Assert-EditorRecoveryExtraAffinity $redone ([pscustomobject]@{selection_affinity='After'}) ($Phase+' Redo')
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    $undone=Assert-EditorRecoverySelection $Baseline.source $Baseline.document_id $Baseline.selection.anchor $Baseline.selection.head $Baseline.dirty $Baseline.undo_count 1 ($Phase+' final Undo returns exact baseline')
    Assert-EditorRecoveryExtraAffinity $undone $Baseline ($Phase+' final Undo')
    return $undone
}

function Invoke-EditorRecoveryLiteralF6Traversal($Baseline,$CurrentState,[string]$ExpectedLiteral,[string]$CaseId,[string]$ProfilePattern) {
    if ($ProfilePattern -notin @('^ATOK','^Microsoft IME')) {
        throw 'SETUP_FAILURE: literal F6 traversal requires an explicit ATOK or Microsoft IME profile pattern.'
    }
    $graphemeCount=[Globalization.StringInfo]::ParseCombiningCharacters($ExpectedLiteral).Count
    if ($graphemeCount -le 0) { throw 'SETUP_FAILURE: F6 phrase traversal needs a nonempty fixed literal oracle.' }
    $maxPhraseAdvances=[Math]::Min(16,[Math]::Max(0,$graphemeCount-1))
    $state=$CurrentState
    Assert-EditorRecoveryExtraInvariant $state $Baseline 'F6 phrase traversal baseline'
    if ([long]$state.revision -ne [long]$Baseline.revision) {
        throw 'PRODUCT_ASSERTION: F6 phrase traversal started from a changed revision.'
    }
    $profile=Get-EditorRecoveryProfileIdentity $state.active_input_profile
    if ($null -eq $profile -or [string]$profile.description -notmatch $ProfilePattern -or
        -not (Test-EditorRecoveryProfileIdentity $Baseline.active_input_profile $state.active_input_profile)) {
        throw 'ENVIRONMENT_BLOCKED: F6 phrase traversal is not running in its explicitly selected profile.'
    }
    if (-not [bool]$state.ime.active -or [string]::IsNullOrEmpty([string]$state.ime.preedit)) {
        throw 'PRODUCT_ASSERTION: F6 phrase traversal requires an active reconversion composition.'
    }

    $isAtok=$ProfilePattern -ceq '^ATOK'
    $progression=@()
    $before=[string]$state.ime.preedit
    Send-Key $script:InputWindowHandle 0x75
    $initialDescription=if($isAtok){'ATOK initial F6 literal phrase conversion'}else{'Microsoft IME initial F6 literal phrase conversion'}
    $state=Wait-EditorRecoveryStableState -WaitMs 10000 -Description $initialDescription -Condition {
        param($s)
        Assert-EditorRecoveryExtraInvariant $s $Baseline 'initial F6 literal phrase conversion'
        if ([long]$s.revision -ne [long]$Baseline.revision) { throw 'PRODUCT_ASSERTION: F6 phrase conversion changed revision.' }
        if (-not [bool]$s.ime.active) { throw 'PRODUCT_ASSERTION: F6 phrase conversion ended composition.' }
        -not [string]::IsNullOrEmpty([string]$s.ime.preedit) -and (Test-EditorRecoveryJapaneseText ([string]$s.ime.preedit))
    }
    $progression+=@{step=0;action='F6';preedit_before=$before;preedit_after=[string]$state.ime.preedit;exact=([string]$state.ime.preedit -ceq $ExpectedLiteral)}
    Add-Observation ($initialDescription) $state
    $initialPhase=if($isAtok){'extra-atok-literal-f6-0'}else{'extra-ms-literal-f6-0'}
    [void](Save-EditorRecoveryEvidence $CaseId $initialPhase)
    if ([string]$state.ime.preedit -ceq $ExpectedLiteral) {
        return [pscustomobject]@{status='EXACT';state=$state;progression=@($progression);max_phrase_advances=$maxPhraseAdvances}
    }
    if (-not $isAtok) {
        $seenPreedits = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        [void]$seenPreedits.Add([string]$state.ime.preedit)
    }

    $phraseAdvance=0
    if ($isAtok) {
        while ([string]$state.ime.preedit -cne $ExpectedLiteral) {
            if ($phraseAdvance -ge $maxPhraseAdvances) {
                throw ('PRODUCT_ASSERTION: ATOK F6 phrase traversal exhausted its {0}-advance fixture bound before the exact literal oracle; no text was committed.' -f $maxPhraseAdvances)
            }
            $phraseAdvance++
            $before=[string]$state.ime.preedit
            Send-Key $script:InputWindowHandle 0x27 ([uint16[]]@(0x10))
            $state=Wait-EditorRecoveryStableState -WaitMs 5000 -Description ('ATOK literal phrase Shift+Right '+$phraseAdvance) -Condition {
                param($s)
                Assert-EditorRecoveryExtraInvariant $s $Baseline ('ATOK literal phrase Shift+Right '+$phraseAdvance)
                if ([long]$s.revision -ne [long]$Baseline.revision) { throw 'PRODUCT_ASSERTION: ATOK phrase navigation changed revision.' }
                if (-not [bool]$s.ime.active) { throw 'PRODUCT_ASSERTION: ATOK phrase navigation ended composition.' }
                -not [string]::IsNullOrEmpty([string]$s.ime.preedit) -and (Test-EditorRecoveryJapaneseText ([string]$s.ime.preedit))
            }
            $progression+=@{step=$phraseAdvance;action='Shift+Right';preedit_before=$before;preedit_after=[string]$state.ime.preedit;exact=([string]$state.ime.preedit -ceq $ExpectedLiteral)}
            Add-Observation ('ATOK literal phrase Shift+Right '+$phraseAdvance) $state
            [void](Save-EditorRecoveryEvidence $CaseId ('extra-atok-literal-shift-right-'+$phraseAdvance))

            $before=[string]$state.ime.preedit
            Send-Key $script:InputWindowHandle 0x75
            $state=Wait-EditorRecoveryStableState -WaitMs 10000 -Description ('ATOK literal F6 phrase '+$phraseAdvance) -Condition {
                param($s)
                Assert-EditorRecoveryExtraInvariant $s $Baseline ('ATOK literal F6 phrase '+$phraseAdvance)
                if ([long]$s.revision -ne [long]$Baseline.revision) { throw 'PRODUCT_ASSERTION: ATOK F6 phrase conversion changed revision.' }
                if (-not [bool]$s.ime.active) { throw 'PRODUCT_ASSERTION: ATOK F6 phrase conversion ended composition.' }
                $preedit=[string]$s.ime.preedit
                -not [string]::IsNullOrEmpty($preedit) -and (Test-EditorRecoveryJapaneseText $preedit)
            }
            $progression+=@{step=$phraseAdvance;action='F6';preedit_before=$before;preedit_after=[string]$state.ime.preedit;exact=([string]$state.ime.preedit -ceq $ExpectedLiteral)}
            Add-Observation ('ATOK literal F6 phrase '+$phraseAdvance) $state
            [void](Save-EditorRecoveryEvidence $CaseId ('extra-atok-literal-f6-'+$phraseAdvance))
        }
    } else {
        while ([string]$state.ime.preedit -cne $ExpectedLiteral) {
            if ($phraseAdvance -ge $maxPhraseAdvances) {
                return [pscustomobject]@{status='BOUND_REACHED';state=$state;progression=@($progression);max_phrase_advances=$maxPhraseAdvances}
            }
            $phraseAdvance++
            $before=[string]$state.ime.preedit
            Send-Key $script:InputWindowHandle 0x27
            $state=Wait-EditorRecoveryStableState -WaitMs 5000 -Description ('Microsoft IME literal phrase Right '+$phraseAdvance) -Condition {
                param($s)
                Assert-EditorRecoveryExtraInvariant $s $Baseline ('Microsoft IME literal phrase Right '+$phraseAdvance)
                if ([long]$s.revision -ne [long]$Baseline.revision) { throw 'PRODUCT_ASSERTION: Microsoft IME phrase navigation changed revision.' }
                if (-not [bool]$s.ime.active) { throw 'PRODUCT_ASSERTION: Microsoft IME phrase navigation ended composition.' }
                -not [string]::IsNullOrEmpty([string]$s.ime.preedit) -and (Test-EditorRecoveryJapaneseText ([string]$s.ime.preedit))
            }
            $afterRight=[string]$state.ime.preedit
            Add-Observation ('Microsoft IME literal phrase Right '+$phraseAdvance) $state
            [void](Save-EditorRecoveryEvidence $CaseId ('extra-ms-literal-right-'+$phraseAdvance))

            Send-Key $script:InputWindowHandle 0x75
            $state=Wait-EditorRecoveryStableState -WaitMs 10000 -Description ('Microsoft IME literal F6 phrase '+$phraseAdvance) -Condition {
                param($s)
                Assert-EditorRecoveryExtraInvariant $s $Baseline ('Microsoft IME literal F6 phrase '+$phraseAdvance)
                if ([long]$s.revision -ne [long]$Baseline.revision) { throw 'PRODUCT_ASSERTION: Microsoft IME F6 phrase conversion changed revision.' }
                if (-not [bool]$s.ime.active) { throw 'PRODUCT_ASSERTION: Microsoft IME F6 phrase conversion ended composition.' }
                $preedit=[string]$s.ime.preedit
                -not [string]::IsNullOrEmpty($preedit) -and (Test-EditorRecoveryJapaneseText $preedit)
            }
            $afterPair=[string]$state.ime.preedit
            $repeated = -not $seenPreedits.Add($afterPair)
            $progression+=@{step=$phraseAdvance;action='Right+F6';preedit_before=$before;preedit_after_right=$afterRight;preedit_after=$afterPair;exact=($afterPair -ceq $ExpectedLiteral);changed=($afterPair -cne $before);repeated=$repeated}
            Add-Observation ('Microsoft IME literal Right+F6 pair '+$phraseAdvance) $state
            [void](Save-EditorRecoveryEvidence $CaseId ('extra-ms-literal-f6-'+$phraseAdvance))
            if ($repeated) {
                return [pscustomobject]@{status='STOPPED_REPEATED_PREEDIT';state=$state;progression=@($progression);max_phrase_advances=$maxPhraseAdvances}
            }
        }
    }
    return [pscustomobject]@{status='EXACT';state=$state;progression=@($progression);max_phrase_advances=$maxPhraseAdvances}
}

function Invoke-EditorRecoveryAtokLongSpaceTraversal($Baseline,[string]$ExpectedReading,[string]$CaseId) {
    $maxSpaces=3
    $state=$Baseline
    $progression=@()
    for ($spaceIndex=1;$spaceIndex -le $maxSpaces;$spaceIndex++) {
        $before=[string]$state.ime.preedit
        Send-Key $script:InputWindowHandle 0x20
        $state=Wait-EditorRecoveryStableState -WaitMs 10000 -Description ('ATOK long Space '+$spaceIndex) -Condition {
            param($s)
            Assert-EditorRecoveryExtraInvariant $s $Baseline ('ATOK long Space '+$spaceIndex)
            if ([long]$s.revision -ne [long]$Baseline.revision) {
                throw 'PRODUCT_ASSERTION: ATOK long Space conversion changed revision.'
            }
            if (-not [bool]$s.ime.active) { throw 'PRODUCT_ASSERTION: ATOK long Space conversion ended composition.' }
            -not [string]::IsNullOrEmpty([string]$s.ime.preedit) -and
                (Test-EditorRecoveryJapaneseText ([string]$s.ime.preedit))
        }
        $progression+=@{step=$spaceIndex;action='Space';preedit_before=$before;preedit_after=[string]$state.ime.preedit;distinct=([string]$state.ime.preedit -cne $ExpectedReading)}
        Add-Observation ('ATOK long Space '+$spaceIndex) $state
        [void](Save-EditorRecoveryEvidence $CaseId ('extra-atok-long-space-'+$spaceIndex))
        if ([string]$state.ime.preedit -cne $ExpectedReading) {
            return [pscustomobject]@{state=$state;progression=@($progression);space_actions=$spaceIndex;max_spaces=$maxSpaces}
        }
    }
    throw ('PRODUCT_ASSERTION: ATOK Space still exposed the original long reading after {0} bounded actions; distinct-candidate conversion PASS was withheld.' -f $maxSpaces)
}

function Invoke-EditorRecoveryLongReadingByWord(
    $Baseline,[string]$Source,[long]$DocumentId,[int]$Tabs,[int]$Undo,[int]$Redo,[string]$ExpectedReading,[string]$CaseId,
    [ValidateSet('first','second')][string]$Phase
) {
    if([string]$Baseline.source -cne $Source -or $ExpectedReading -cne ('にほんご'*20)){
        throw 'SETUP_FAILURE: word-step reading requires the exact preexisting Core baseline and fixed eighty-character oracle.'
    }
    $profile=Get-EditorRecoveryProfileIdentity $Baseline.active_input_profile
    $progression=[Collections.Generic.List[object]]::new()
    $state=$null
    for($wordIndex=1;$wordIndex -le 20;$wordIndex++){
        $expectedPrefix='にほんご'*$wordIndex
        Send-Ascii 'nihongo' $script:InputWindowHandle
        $state=Wait-EditorRecoveryStableState -WaitMs 10000 -Description (($Phase+' long reading word ')+$wordIndex+'/20') -Condition {
            param($s)
            Assert-EditorRecoveryExtraInvariant $s $Baseline (($Phase+' long reading word ')+$wordIndex)
            $s.source -ceq $Source -and [long]$s.document_id -eq $DocumentId -and [int]$s.tabs -eq $Tabs -and
                [long]$s.revision -eq [long]$Baseline.revision -and [bool]$s.dirty -eq [bool]$Baseline.dirty -and
                [int]$s.undo_count -eq $Undo -and [int]$s.redo_count -eq $Redo -and
                [int]$s.selection.anchor -eq [int]$Baseline.selection.anchor -and [int]$s.selection.head -eq [int]$Baseline.selection.head -and
                [string]$s.selection_affinity -ceq [string]$Baseline.selection_affinity -and -not [bool]$s.raw -and
                (Test-EditorRecoveryProfileIdentity $profile $s.active_input_profile) -and [bool]$s.ime.open_status.open -and
                [bool]$s.ime.active -and $s.ime.preedit -ceq $expectedPrefix
        }
        Add-Observation (($Phase+' long reading exact word prefix ')+$wordIndex+'/20') $state
        $progression.Add([pscustomobject]@{
            phase=$Phase;word_index=$wordIndex;expected_preedit=$expectedPrefix;observed_preedit=[string]$state.ime.preedit
            source_sha256=Get-UiTextSha256 ([string]$state.source);document_id=[long]$state.document_id;tabs=[int]$state.tabs
            revision=[long]$state.revision;dirty=[bool]$state.dirty;undo_count=[int]$state.undo_count;redo_count=[int]$state.redo_count
            selection=$state.selection;selection_affinity=[string]$state.selection_affinity
            active_profile=Get-EditorRecoveryProfileIdentity $state.active_input_profile;app_local_open=[bool]$state.ime.open_status.open
        })
    }
    if($null -eq $state -or [string]$state.ime.preedit -cne $ExpectedReading -or $progression.Count -ne 20){
        throw 'PRODUCT_ASSERTION: long word-step route did not reach the exact twenty-word reading.'
    }
    [pscustomobject]@{progression=@($progression.ToArray())}
}

function Invoke-EditorRecoveryImeExtra(
    [string]$CaseId,[string]$Pattern,[string]$CommittedSuffix,[string]$LiteralReading
) {
    $script:CurrentEditorRecoveryCaseId=$CaseId
    $case=@($script:Report.editor_recovery_cases | Where-Object id -ceq $CaseId)[0]
    $source=[string]$script:EditorRecoveryExpectedSource
    if ([string]::IsNullOrEmpty($CommittedSuffix) -or [string]::IsNullOrEmpty($LiteralReading) -or
        -not $source.EndsWith($CommittedSuffix,[StringComparison]::Ordinal)) {
        throw 'SETUP_FAILURE: IME extra branch lacks a validated core suffix/reading oracle.'
    }
    $initial=Switch-ToSource $source
    Assert-EditorRecoveryExtraAffinity $initial $initial 'extra entry affinity metadata'
    $id=[long]$initial.document_id; $tabs=[int]$initial.tabs
    $initialProfile=Get-EditorRecoveryProfileIdentity $initial.active_input_profile
    if (-not [bool]$initial.ime.open_status.available -or [bool]$initial.ime.active -or -not [string]::IsNullOrEmpty([string]$initial.ime.preedit)) {
        throw 'ENVIRONMENT_BLOCKED: extra IME branch needs observable inactive initial composition.'
    }
    $initialOpen=[bool]$initial.ime.open_status.open
    $extra=[ordered]@{
        reconversion='NOT_RUN';literal_replacement='NOT_RUN';phrase_focus='NOT_RUN';phrase_boundary='NOT_RUN'
        global_key_template='UNVERIFIED';tested_operational_routes=@();long_reading='NOT_RUN';long_space_progression='NOT_RUN';long_conversion_cancel='NOT_RUN';long_literal_f6='NOT_RUN';long_literal_f6_note='NOT_APPLICABLE';long_literal_commit='NOT_RUN'
        microsoft_literal_f6_progression=@();microsoft_literal_f6_status='NOT_RUN'
        first_long_reading_word_route='NOT_RUN';first_long_reading_word_progression=@()
        second_long_reading_word_route='NOT_RUN';second_long_reading_word_progression=@()
        wrapped_geometry='NOT_RUN';candidate_ui_visual_review='NOT_RUN';physical_input='NOT_RUN'
    }
    if ($Pattern -ceq 'ATOK') {
        $extra.long_literal_f6='NOT_ACCEPTED_SEPARATE_RAW80_DIAGNOSTIC'
        $extra.long_literal_f6_note='Separate raw-80 F6 observation changed two characters; provider reason is unproven and this is not an acceptance result.'
    }
    $case.details['extra_ime']=$extra
    $primary=$null;$profileReady=$false
    try {
        $profile=Request-EditorRecoveryProfile ($CaseId+'-extra') $Pattern
        if ($null -eq $profile) { throw 'ENVIRONMENT_BLOCKED: requested installed profile did not reach the owned editor.' }
        $script:ImeProfile=$profile.active_input_profile
        $script:ImeInitialOpen=$initialOpen; $script:ImeRestorePending=$true; $script:ImeRestoreAttempted=$false
        $script:ImeEscapesSentForComposition=0;$profileReady=$true
        if (-not [bool]$profile.ime.open_status.available) { throw 'ENVIRONMENT_BLOCKED: selected profile has no observable owned IME open state.' }
        if (-not [bool]$profile.ime.open_status.open) { Send-Key $script:InputWindowHandle 0x16; [void](Wait-ImeOpenState $true $source) }
        $state=Read-Diagnostic
        if (-not (Test-SameInputProfile $state) -or [int]$state.tabs -ne $tabs) { throw 'SETUP_FAILURE: extra IME activation changed its pinned profile or tab ownership.' }
        [void](Assert-EditorRecoverySelection $source $id $initial.selection.anchor $initial.selection.head $initial.dirty $initial.undo_count $initial.redo_count ($CaseId+' profile activation preserves Core state'))
        $u=[int]$initial.undo_count; $redo=[int]$initial.redo_count
        $selected=Select-EditorRecoveryCommittedSuffix $source $id $CommittedSuffix ([bool]$initial.dirty) $u $redo $CaseId 'reconversion'
        $probe=Invoke-EditorRecoveryReconversionProbe $selected $Pattern $extra
        if ($null -ne $probe) {
            $converted=$probe.state
            $extra.reconversion='PASS_COMPOSITION_SOURCE_DIRECTION_AFFINITY_HISTORY_INVARIANTS'
            [void](Save-EditorRecoveryEvidence $CaseId 'extra-reconversion-active')
            # Successful reconversion keys do not prove a global template.
            # Exercise the corresponding documented phrase pair, then require
            # actual target/boundary highlight review before any phrase PASS.
            $focusModifiers=if ($probe.route -ceq 'Shift+Convert') {[uint16[]]@(0x10)}else{[uint16[]]@()}
            $boundaryModifiers=if ($probe.route -ceq 'Shift+Convert') {[uint16[]]@()}else{[uint16[]]@(0x10)}
            $extra.documented_phrase_route=$probe.route
            foreach ($kind in @('phrase-focus','phrase-boundary')) {
                $keys=if ($kind -ceq 'phrase-focus') {$focusModifiers}else{$boundaryModifiers}
                foreach ($key in @(0x25,0x27)) {
                    Send-Key $script:InputWindowHandle $key $keys
                    $converted=Wait-EditorRecoveryStableState -WaitMs 5000 -Description ($kind+' documented operation; template unverified') -Condition {
                        param($s)
                        Assert-EditorRecoveryExtraInvariant $s $selected $kind
                        [bool]$s.ime.active -and (Test-EditorRecoveryJapaneseText ([string]$s.ime.preedit))
                    }
                    [void](Save-EditorRecoveryEvidence $CaseId ('extra-'+$kind+'-'+$key))
                }
            }
            $extra.phrase_focus='NOT_RUN_TARGET_HIGHLIGHT_REVIEW';$extra.phrase_boundary='NOT_RUN_BOUNDARY_HIGHLIGHT_REVIEW'
            if ($Pattern -ceq 'ATOK') {
                $cancel=Cancel-EditorRecoveryImeComposition $selected $converted 'extra reconversion and phrase navigation' 3 -AllowAtokReconversionSelectionCollapse
            } else {
                $maxEscapes = Get-EditorRecoveryImeCancellationEscapeLimit $converted.active_input_profile
                $cancel=Cancel-EditorRecoveryImeComposition $selected $converted 'extra reconversion and phrase navigation' $maxEscapes
            }
            if (-not $cancel.cancelled) { throw 'ENVIRONMENT_BLOCKED: reconversion remained active after the bounded ordinary Escape sequence.' }
            if ($Pattern -ceq 'ATOK') {
                Assert-EditorRecoveryAtokReconversionCancelInvariant $cancel.state $selected 'ATOK reconversion Esc'
                Add-Observation 'ATOK reconversion Esc preserves source, ownership, revision, dirty/history; selection preserved or collapsed' $cancel.state
            } else {
                Assert-EditorRecoveryExtraInvariant $cancel.state $selected 'reconversion Esc'
                [void](Assert-EditorRecoverySelection $source $id $selected.selection.anchor $selected.selection.head $selected.dirty $u $redo 'reconversion Esc preserves exact source/direction/history')
            }
            # Fixed alternate spelling makes replacement meaningful even when
            # the core's observed candidate is already the literal hiragana.
            $literal=$LiteralReading;$literalKey=0x75;$literalName='F6 hiragana'
            if ($CommittedSuffix -ceq $LiteralReading) {
                $literalKey=0x76;$literalName='F7 fullwidth katakana'
                $literal=switch -CaseSensitive ($LiteralReading) {
                    'にほんご' {'ニホンゴ'}
                    'にほんごきょうはいめへんか' {'ニホンゴキョウハイメヘンカ'}
                    default {throw 'SETUP_FAILURE: no independently fixed katakana oracle for this reading.'}
                }
            }
            if ($literal -ceq $CommittedSuffix) { throw 'SETUP_FAILURE: literal replacement oracle must be distinct.' }
            $extra.literal_key=$literalName;$extra.literal_oracle=$literal
            $selected=Select-EditorRecoveryCommittedSuffix $source $id $CommittedSuffix ([bool]$initial.dirty) $u $redo $CaseId 'literal reconversion'
            $script:ImeEscapesSentForComposition=0
            Send-Key $script:InputWindowHandle 0x1C $probe.modifiers
            $literalComposition=Wait-EditorRecoveryStableState -WaitMs 5000 -Description 'repeat observed operational reconversion route' -Condition {
                param($s)
                Assert-EditorRecoveryExtraInvariant $s $selected 'repeat reconversion'
                [bool]$s.ime.active -and (Test-EditorRecoveryJapaneseText ([string]$s.ime.preedit))
            }
            if ($Pattern -ceq 'ATOK' -and $literalKey -eq 0x75) {
                $literalTraversal=Invoke-EditorRecoveryLiteralF6Traversal $selected $literalComposition $literal $CaseId '^ATOK'
                if ([string]$literalTraversal.status -cne 'EXACT') { throw 'PRODUCT_ASSERTION: ATOK literal F6 did not reach its fixed oracle.' }
                $extra.literal_progression=@($literalTraversal.progression)
            } elseif ($Pattern -ceq 'Microsoft IME' -and $literalKey -eq 0x75) {
                $literalTraversal=Invoke-EditorRecoveryLiteralF6Traversal $selected $literalComposition $literal $CaseId '^Microsoft IME'
                $extra.microsoft_literal_f6_progression=@($literalTraversal.progression)
                if ([string]$literalTraversal.status -ceq 'EXACT') {
                    $extra.microsoft_literal_f6_status='PASS_FULL_FIXED_ORACLE'
                } else {
                    $extra.microsoft_literal_f6_status='BLOCKED_'+[string]$literalTraversal.status
                    $extra.literal_replacement='NOT_RUN_F6_ORACLE_INCOMPLETE'
                    throw 'ENVIRONMENT_BLOCKED: Microsoft IME Right+F6 phrase traversal did not reach the exact full literal.'
                }
            } else {
                Send-Key $script:InputWindowHandle $literalKey
                [void](Wait-EditorRecoveryStableState -WaitMs 10000 -Description ('independently fixed '+$literalName) -Condition {
                    param($s)
                    Assert-EditorRecoveryExtraInvariant $s $selected 'literal reconversion'
                    [bool]$s.ime.active -and $s.ime.preedit -ceq $literal
                })
            }
            $prefix=$source.Substring(0,$source.Length-$CommittedSuffix.Length)
            $replacement=$prefix+$literal
            [void](Invoke-EditorRecoveryLiteralImeCommitCycle $selected $replacement ([Text.Encoding]::UTF8.GetByteCount($replacement)) $CaseId ('reconverted '+$literalName))
            $extra.literal_replacement='PASS_DISTINCT_SOURCE_ONE_UNDO_REDO_FINAL_UNDO';$redo=1
        } else {
            $extra.reconversion='BLOCKED_DOCUMENTED_ROUTES_NO_JAPANESE_COMPOSITION'
            $extra.literal_replacement='NOT_RUN_RECONVERSION_UNPROVEN'
        }
        [void](Invoke-EditorViewportBodyClick $CaseId 'extra-long-reading native focus')
        $mode=Read-Diagnostic
        if (-not (Test-SameInputProfile $mode) -or -not [bool]$mode.ime.open_status.available -or
            [bool]$mode.ime.active -or -not [string]::IsNullOrEmpty([string]$mode.ime.preedit)) {
            throw 'ENVIRONMENT_BLOCKED: long reading requires the pinned inactive profile and observable owned open state.'
        }
        # Convert can be an app-local mode key under another documented
        # assignment. Re-establish normal Hiragana input without changing keys.
        if (-not [bool]$mode.ime.open_status.open) { Send-Key $script:InputWindowHandle 0x16; [void](Wait-ImeOpenState $true $source) }
        Send-Key $script:InputWindowHandle 0x23 ([uint16[]]@(0x11))
        $end=[Text.Encoding]::UTF8.GetByteCount($source)
        $baseline=Assert-EditorRecoverySelection $source $id $end $end ([bool]$initial.dirty) $u $redo 'long-reading exact Core EOF'
        $romaji=('nihongo'*20);$hiragana=('にほんご'*20);$script:ImeEscapesSentForComposition=0
        if ($Pattern -ceq 'Microsoft IME') {
            $firstWordTraversal=Invoke-EditorRecoveryLongReadingByWord $baseline $source $id $tabs $u $redo $hiragana $CaseId -Phase 'first'
            $extra.first_long_reading_word_progression=@($firstWordTraversal.progression)
            $extra.first_long_reading_word_route='PASS_20_CANONICAL_STABLE_PREFIXES'
        } else {
            Send-Ascii $romaji $script:InputWindowHandle
        }
        $reading=Wait-EditorRecoveryStableState -WaitMs 10000 -Description 'complete eighty-character Japanese reading' -Condition {
            param($s)
            Assert-EditorRecoveryExtraInvariant $s $baseline 'long reading'
            [long]$s.document_id -eq $id -and [int]$s.tabs -eq $tabs -and [bool]$s.ime.active -and $s.ime.preedit -ceq $hiragana
        }
        $extra.long_reading='PASS_EXACT_RAW_READING';[void](Save-EditorRecoveryEvidence $CaseId 'extra-long-reading')
        $point=$reading.PSObject.Properties['composition_caret'];$basePoint=$baseline.PSObject.Properties['caret']
        if ($null -ne $point -and $point.Value -is [PSCustomObject] -and $null -ne $basePoint -and $basePoint.Value -is [PSCustomObject]) {
            $complete=$true
            foreach ($field in @('x','y','height')) {
                if ($null -eq $point.Value.PSObject.Properties[$field] -or $null -eq $basePoint.Value.PSObject.Properties[$field]) {$complete=$false;break}
                Assert-EditorRecoveryFiniteGeometryNumber $point.Value.PSObject.Properties[$field].Value ('long composition '+$field)
                Assert-EditorRecoveryFiniteGeometryNumber $basePoint.Value.PSObject.Properties[$field].Value ('long baseline '+$field)
            }
            if ($complete -and [double]$point.Value.y -gt ([double]$basePoint.Value.y+[double]$basePoint.Value.height)) {
                $extra.wrapped_geometry='MULTILINE_CARET_HINT_VISUAL_REVIEW_NOT_RUN'
            }
        }
        if ($Pattern -ceq 'ATOK') {
            $longSpaceTraversal=Invoke-EditorRecoveryAtokLongSpaceTraversal $baseline $hiragana $CaseId
            $converted=$longSpaceTraversal.state
            $extra.long_space_progression=@($longSpaceTraversal.progression)
        } else {
            Send-Key $script:InputWindowHandle 0x20
            $converted=Wait-EditorRecoveryStableState -WaitMs 10000 -Description 'long stable normal Space conversion' -Condition {
                param($s)
                Assert-EditorRecoveryExtraInvariant $s $baseline 'long Space conversion'
                [long]$s.document_id -eq $id -and [int]$s.tabs -eq $tabs -and [bool]$s.ime.active -and
                    [string]$s.ime.preedit -cne $hiragana -and (Test-EditorRecoveryJapaneseText ([string]$s.ime.preedit))
            }
        }
        [void](Save-EditorRecoveryEvidence $CaseId 'extra-long-conversion')
        $maxEscapes = Get-EditorRecoveryImeCancellationEscapeLimit $converted.active_input_profile
        $cancel=Cancel-EditorRecoveryImeComposition $baseline $converted 'extra long conversion' $maxEscapes
        if (-not $cancel.cancelled) { throw 'ENVIRONMENT_BLOCKED: long conversion remained active after the bounded ordinary Escape sequence.' }
        Assert-EditorRecoveryExtraInvariant $cancel.state $baseline 'long conversion Esc'
        [void](Assert-EditorRecoverySelection $source $id $end $end ([bool]$initial.dirty) $u $redo 'long conversion Esc preserves exact source/direction/history')
        $extra.long_conversion_cancel='PASS_SOURCE_DIRECTION_HISTORY_UNCHANGED';$script:ImeEscapesSentForComposition=0
        if ($Pattern -ceq 'Microsoft IME') {
            $secondWordTraversal=Invoke-EditorRecoveryLongReadingByWord $baseline $source $id $tabs $u $redo $hiragana $CaseId -Phase 'second'
            $extra.second_long_reading_word_progression=@($secondWordTraversal.progression)
            $extra.second_long_reading_word_route='PASS_20_CANONICAL_STABLE_PREFIXES'
        } else {
            Send-Ascii $romaji $script:InputWindowHandle
        }
        $longLiteralReady=Wait-EditorRecoveryStableState -WaitMs 10000 -Description 'second exact long reading before literal commit' -Condition {
            param($s)
            Assert-EditorRecoveryExtraInvariant $s $baseline 'second long reading'
            [long]$s.document_id -eq $id -and [int]$s.tabs -eq $tabs -and [bool]$s.ime.active -and $s.ime.preedit -ceq $hiragana
        }
        Add-Observation 'long exact reading directly before literal commit' $longLiteralReady
        [void](Save-EditorRecoveryEvidence $CaseId 'extra-long-literal-before-commit')
        [void](Invoke-EditorRecoveryLiteralImeCommitCycle $baseline ($source+$hiragana) ($end+240) $CaseId 'long fixed hiragana')
        $extra.long_literal_commit='PASS_SOURCE_ONE_UNDO_REDO_FINAL_UNDO'
        $extra.final_source_sha256=Get-UiTextSha256 $source;$extra.final_undo=$u;$extra.final_redo=1
    } catch { $primary=$_.Exception; throw }
    finally {
        try {
            # Cancel via the existing max-three-Escape restoration first while
            # the selected provider identity is still current, preserving error.
            if ($profileReady) { Restore-ImeOpenState }
            $fresh=Read-Diagnostic
            if (-not $profileReady -and ([bool]$fresh.ime.active -or -not [string]::IsNullOrEmpty([string]$fresh.ime.preedit))) {
                throw 'ENVIRONMENT_BLOCKED: failed profile selection left an unvalidated active composition; scoped cleanup sent no key.'
            }
            if (-not (Test-EditorRecoveryProfileIdentity $initialProfile $fresh.active_input_profile)) {
                $fresh=Request-EditorRecoveryProfile ($CaseId+'-extra-restore') $null $initialProfile
                if ($null -eq $fresh) { throw 'ENVIRONMENT_BLOCKED: initial profile restoration after extra branch failed.' }
            }
            $script:ImeProfile=$fresh.active_input_profile;$script:ImeInitialOpen=$initialOpen
            $script:ImeRestorePending=$true;$script:ImeRestoreAttempted=$false
            Restore-ImeOpenState
            $restored=Read-Diagnostic
            if (-not (Test-EditorRecoveryProfileIdentity $initialProfile $restored.active_input_profile) -or
                -not [bool]$restored.ime.open_status.available -or [bool]$restored.ime.open_status.open -ne $initialOpen) {
                throw 'ENVIRONMENT_BLOCKED: extra IME branch did not restore its entry profile and observable open state.'
            }
            $extra.profile_and_open_restoration='PASS'
        } catch { if ($null -eq $primary) {throw};$script:Report.ime_open_restoration_error=$_.Exception.Message }
    }
}

function Restore-EditorRecoveryInitialImeStateForFailure {
    $blockedSaveAsCleanup = @($script:Report.save_as_dialog_cleanup | Where-Object {
        $_.status -ceq 'BLOCKED' -and $_.dialog_still_present
    })
    if ($blockedSaveAsCleanup.Count -gt 0) {
        return @{
            status='BLOCKED'; restored_in_failure_finally=$false
            reason='Owned SaveAs cancellation was not confirmed; profile/open-state restoration was not attempted while its modal may still hold the App borrow.'
            save_as_cleanup=$blockedSaveAsCleanup[-1]
        }
    }
    $initialProfile = $script:Report.active_input_profile_initial
    $initialOpenStatus = $script:Report.initial_app_local_ime_open_status
    if ($null -eq $initialProfile -or $null -eq $initialOpenStatus -or $null -eq $initialOpenStatus.open) {
        return @{ status='NOT_RUN'; restored_in_failure_finally=$false; reason='Initial profile and app-local open state were not both captured.' }
    }
    if ($null -eq $script:Process -or $script:WindowHandle -eq [IntPtr]::Zero) {
        if ($null -eq $script:Process -and $script:WindowHandle -eq [IntPtr]::Zero) {
            $normalCloseRecords = @($script:Report.normal_close)
            if ($normalCloseRecords.Count -gt 0) {
                $normalClose = $normalCloseRecords[-1]
                $preclose = $normalClose.ime_restoration_preclose
                $precloseRestoration = if ($preclose) { $preclose.restoration } else { $null }
                $ownerMatches = $null -ne $preclose -and $null -ne $precloseRestoration -and
                    [string]$normalClose.stage -ceq 'reopen-verification' -and
                    [string]$normalClose.status -ceq 'PASS' -and
                    [string]$normalClose.close_request -ceq 'normal Alt+F4' -and
                    [bool]$normalClose.wait_completed -and
                    -not [bool]$normalClose.process_still_running -and
                    $null -ne $normalClose.exit_code -and
                    [int]$normalClose.exit_code -eq 0 -and
                    $null -ne $normalClose.pid -and
                    $null -ne $preclose.owner_pid -and
                    [int]$normalClose.pid -eq [int]$preclose.owner_pid -and
                    -not [string]::IsNullOrEmpty([string]$normalClose.owner_start_time_utc) -and
                    -not [string]::IsNullOrEmpty([string]$preclose.owner_start_time_utc) -and
                    [string]$normalClose.owner_start_time_utc -ceq [string]$preclose.owner_start_time_utc -and
                    -not [string]::IsNullOrEmpty([string]$normalClose.owner_hwnd) -and
                    -not [string]::IsNullOrEmpty([string]$preclose.owner_hwnd) -and
                    [string]$normalClose.owner_hwnd -ceq [string]$preclose.owner_hwnd -and
                    [bool]$preclose.restoration_attempted_while_owner_live -and
                    [string]$preclose.scope -ceq 'final IME restoration attempt on the owned live process before normal Alt+F4'
                $statusMatches = $ownerMatches -and
                    [string]$precloseRestoration.status -in @('PASS','BLOCKED','NOT_RUN')
                if ($statusMatches -and [string]$precloseRestoration.status -ceq 'PASS') {
                    $statusMatches = [bool]$precloseRestoration.source_selection_dirty_history_preserved -and
                        (Test-EditorRecoveryProfileIdentity $initialProfile $precloseRestoration.initial_profile) -and
                        (Test-EditorRecoveryProfileIdentity $initialProfile $precloseRestoration.final_profile) -and
                        $null -ne $precloseRestoration.initial_app_local_open -and
                        $null -ne $precloseRestoration.final_app_local_open -and
                        [bool]$precloseRestoration.initial_app_local_open -eq [bool]$initialOpenStatus.open -and
                        [bool]$precloseRestoration.final_app_local_open -eq [bool]$initialOpenStatus.open
                }
                if ($statusMatches) {
                    $result = $precloseRestoration.Clone()
                    $result.restored_in_failure_finally=$false
                    $result.restored_before_verified_normal_exit=[string]$precloseRestoration.status -ceq 'PASS'
                    $result.verification_scope=if ([string]$precloseRestoration.status -ceq 'PASS') {
                        'Fresh guarded readback immediately before normal Alt+F4; the same owned process then exited with code 0. No post-close readback was performed.'
                    } else {
                        'Restoration remained {0} before normal Alt+F4; the same owned process then exited with code 0. No passing post-close readback is claimed.' -f $precloseRestoration.status
                    }
                    $result.owner_pid=[int]$preclose.owner_pid
                    $result.owner_start_time_utc=[string]$preclose.owner_start_time_utc
                    $result.owner_hwnd=[string]$preclose.owner_hwnd
                    return $result
                }
            }
        }
        return @{ status='NOT_RUN'; restored_in_failure_finally=$false; reason='No owned editor process/window remained available for guarded restoration.' }
    }
    try {
        $script:Process.Refresh()
        if ($script:Process.HasExited) {
            return @{ status='NOT_RUN'; restored_in_failure_finally=$false; reason='The owned editor process had already exited.' }
        }
        Assert-OwnedWindow $script:WindowHandle
        $before = Wait-Diagnostic { param($state) $true } 'failure-finally profile restoration baseline'
        if ([bool]$before.ime.active -or -not [string]::IsNullOrEmpty([string]$before.ime.preedit)) {
            if ($null -eq $before.active_input_profile) { throw 'SETUP_FAILURE: active profile was unavailable while preserving an owned IME composition.' }
            $script:ImeProfile = $before.active_input_profile
            $maxEscapes = Get-EditorRecoveryImeCancellationEscapeLimit $before.active_input_profile
            $cancelResult = Cancel-EditorRecoveryImeComposition $before $before 'failure-finally owned composition' $maxEscapes
            if (-not $cancelResult.cancelled) {
                throw 'ENVIRONMENT_BLOCKED: initial profile was not switched because the owned composition remained active after the bounded Escape sequence.'
            }
            $afterCancel = $cancelResult.state
            if ($afterCancel.source -cne $before.source -or
                [int]$afterCancel.selection.anchor -ne [int]$before.selection.anchor -or
                [int]$afterCancel.selection.head -ne [int]$before.selection.head -or
                [bool]$afterCancel.dirty -ne [bool]$before.dirty -or
                [int]$afterCancel.undo_count -ne [int]$before.undo_count -or
                [int]$afterCancel.redo_count -ne [int]$before.redo_count) {
                throw 'PRODUCT_ASSERTION: failure-finally Escape changed source, selection, dirty state, or history.'
            }
            $before = $afterCancel
        }
        $profileRestored = Test-EditorRecoveryProfileIdentity $initialProfile $before.active_input_profile
        if (-not $profileRestored) {
            $requested = Request-EditorRecoveryProfile 'failure-finally-restore' $null $initialProfile
            if ($null -eq $requested) { throw 'SETUP_FAILURE: initial input profile could not be reactivated in the owned Editor.' }
            $before = $requested
        }
        $script:ImeProfile = $initialProfile
        $script:ImeInitialOpen = [bool]$initialOpenStatus.open
        $script:ImeRestorePending = $true
        $script:ImeRestoreAttempted = $false
        Restore-ImeOpenState
        $after = Wait-Diagnostic {
            param($state)
            (Test-EditorRecoveryProfileIdentity $initialProfile $state.active_input_profile) -and
                [bool]$state.ime.open_status.available -and
                [bool]$state.ime.open_status.open -eq [bool]$initialOpenStatus.open -and
                -not [bool]$state.ime.active -and
                [string]::IsNullOrEmpty([string]$state.ime.preedit)
        } 'failure-finally restored profile and app-local open state'
        if ($after.source -cne $before.source -or
            [int]$after.selection.anchor -ne [int]$before.selection.anchor -or
            [int]$after.selection.head -ne [int]$before.selection.head -or
            [bool]$after.dirty -ne [bool]$before.dirty -or
            [int]$after.undo_count -ne [int]$before.undo_count -or
            [int]$after.redo_count -ne [int]$before.redo_count) {
            throw 'PRODUCT_ASSERTION: failure-finally IME restoration changed source, selection, dirty state, or history.'
        }
        return @{
            status='PASS'; restored_in_failure_finally=$true
            initial_profile=$initialProfile
            final_profile=Get-EditorRecoveryProfileIdentity $after.active_input_profile
            initial_app_local_open=[bool]$initialOpenStatus.open
            final_app_local_open=[bool]$after.ime.open_status.open
            source_selection_dirty_history_preserved=$true
        }
    } catch {
        return @{
            status='BLOCKED'; restored_in_failure_finally=$true
            initial_profile=$initialProfile; initial_app_local_open=[bool]$initialOpenStatus.open
            reason=$_.Exception.Message
        }
    }
}

function Close-EditorRecoveryProcess([string]$Label) {
    $captureFinalImeRestoration = $Label -ceq 'reopen-verification'
    $ownerStartTimeUtc = $null
    $ownerHwnd = $null
    $imeRestorationPreclose = $null
    if ($null -eq $script:Process) {
        $script:Report.normal_close += [pscustomobject]@{
            stage=$Label; status='BLOCKED'; pid=$null; close_request='NOT_SENT'
            wait_completed=$false; process_still_running=$null; exit_code=$null
        }
        throw ('OBSERVER_FAILURE: Owned {0} process handle is unavailable before normal Alt+F4; process state is unknown.' -f $Label)
    }
    $process=$script:Process
    $process.Refresh()
    $processId=[int]$process.Id
    if ($process.HasExited) {
        $exitCode=[int]$process.ExitCode
        $script:Report.normal_close += [pscustomobject]@{
            stage=$Label; status='BLOCKED'; pid=$processId; close_request='NOT_SENT'
            wait_completed=$false; process_still_running=$false; exit_code=$exitCode
        }
        throw ('SETUP_FAILURE: Owned {0} process exited before normal Alt+F4 was requested; process_still_running=false; exit_code={1}.' -f $Label,$exitCode)
    }
    if ($captureFinalImeRestoration) {
        try { $ownerStartTimeUtc=$process.StartTime.ToUniversalTime().ToString('o') }
        catch { throw ('OBSERVER_FAILURE: Could not identify the live reopen-verification process start time before IME restoration: {0}' -f $_.Exception.Message) }
        $ownerHwnd='0x{0:X}' -f $script:WindowHandle.ToInt64()
        Assert-OwnedWindow $script:WindowHandle
        $restoration=Restore-EditorRecoveryInitialImeStateForFailure
        $restoration.restored_in_failure_finally=$false
        $restoration.restored_before_normal_close=([string]$restoration.status -ceq 'PASS')
        $process.Refresh()
        if ($process.HasExited) {
            throw 'OBSERVER_FAILURE: The reopen-verification process exited during fresh pre-close IME restoration; normal Alt+F4 was not sent.'
        }
        Assert-OwnedWindow $script:WindowHandle
        $imeRestorationPreclose=@{
            scope='final IME restoration attempt on the owned live process before normal Alt+F4'
            restoration_attempted_while_owner_live=$true
            owner_pid=$processId; owner_start_time_utc=$ownerStartTimeUtc; owner_hwnd=$ownerHwnd
            restoration=$restoration
        }
    }
    Send-Key $script:WindowHandle 0x73 ([uint16[]]@(0x12))
    $waitCompleted=[bool]$process.WaitForExit(15000)
    $process.Refresh()
    $processStillRunning=-not [bool]$process.HasExited
    if (-not $waitCompleted -or $processStillRunning) {
        $exitCode=$null
        if (-not $processStillRunning) { $exitCode=[int]$process.ExitCode }
        $exitCodeText=if ($null -eq $exitCode) {'unavailable'} else {[string]$exitCode}
        $script:Report.normal_close += [pscustomobject]@{
            stage=$Label; status='BLOCKED'; pid=$processId; close_request='normal Alt+F4'
            wait_completed=$waitCompleted; process_still_running=$processStillRunning; exit_code=$exitCode
            owner_start_time_utc=$ownerStartTimeUtc; owner_hwnd=$ownerHwnd
            ime_restoration_preclose=$imeRestorationPreclose
        }
        throw ('OBSERVER_FAILURE: Owned {0} process did not complete normal-close wait within 15000 ms; process_still_running={1}; wait_completed={2}; exit_code={3}.' -f
            $Label,$processStillRunning,$waitCompleted,$exitCodeText)
    }
    $exitCode=[int]$process.ExitCode
    if ($exitCode -ne 0) {
        $script:Report.normal_close += [pscustomobject]@{
            stage=$Label; status='FAIL'; pid=$processId; close_request='normal Alt+F4'
            wait_completed=$true; process_still_running=$false; exit_code=$exitCode
            owner_start_time_utc=$ownerStartTimeUtc; owner_hwnd=$ownerHwnd
            ime_restoration_preclose=$imeRestorationPreclose
        }
        throw ('PRODUCT_ASSERTION: Owned {0} process exited after normal Alt+F4 with nonzero exit code {1}; process_still_running=false.' -f
            $Label,$exitCode)
    }
    $script:Report.normal_close += [pscustomobject]@{
        stage=$Label; status='PASS'; pid=$processId; close_request='normal Alt+F4'
        wait_completed=$true; process_still_running=$false; exit_code=$exitCode
        owner_start_time_utc=$ownerStartTimeUtc; owner_hwnd=$ownerHwnd
        ime_restoration_preclose=$imeRestorationPreclose
    }
    $process.Dispose(); $script:Process=$null
    $script:WindowHandle=[IntPtr]::Zero; $script:InputWindowHandle=[IntPtr]::Zero; $script:ProcessId=[uint32]0
}

function Get-EditorRecoveryBoundaryLocation([string]$Source, [int]$Byte) {
    $utf8=[Text.UTF8Encoding]::new($false,$true)
    $bytes=$utf8.GetBytes($Source)
    if ($Byte -lt 0 -or $Byte -gt $bytes.Length) { throw 'SETUP_FAILURE: fixed byte oracle is outside source.' }
    $prefix=$utf8.GetString($bytes,0,$Byte)
    if ($prefix.EndsWith("`r") -and $Source.Substring($prefix.Length).StartsWith("`n")) {
        throw 'SETUP_FAILURE: fixed byte oracle splits CRLF.'
    }
    $breaks=@([regex]::Matches($prefix,'\r\n|\r|\n'))
    $lineStart=if ($breaks.Count) {$breaks[-1].Index+$breaks[-1].Length} else {0}
    [pscustomobject]@{
        character_index=$prefix.Length; physical_line=$breaks.Count
        right_count=[Globalization.StringInfo]::ParseCombiningCharacters($prefix.Substring($lineStart)).Count
    }
}

function Move-EditorRecoveryBoundaryCaret(
    [string]$Source,[long]$DocumentId,[int]$Byte,[int]$SelectedBytes,
    [bool]$Dirty,[int]$Undo,[int]$Redo,[string]$CaseId,[string]$Description,[int]$ExpectedTabs=2
) {
    $start=Get-EditorRecoveryBoundaryLocation $Source $Byte
    $finish=Get-EditorRecoveryBoundaryLocation $Source ($Byte+$SelectedBytes)
    $state=Switch-ToSource $Source
    if ([long]$state.document_id -ne $DocumentId -or [int]$state.tabs -ne $ExpectedTabs) {
        throw "SETUP_FAILURE: $Description requires document $DocumentId and exactly $ExpectedTabs owned Workspace tabs."
    }
    [void](Invoke-EditorViewportBodyClick $CaseId ($Description+' native body focus'))
    if (-not [bool]$state.raw) {
        Send-Key $script:InputWindowHandle 0x45 ([uint16[]]@(0x11))
        [void](Wait-Diagnostic {param($s) [long]$s.document_id -eq $DocumentId -and $s.source -ceq $Source -and [bool]$s.raw} ($Description+' Source mode'))
    }
    Send-Key $script:InputWindowHandle 0x24 ([uint16[]]@(0x11))
    for ($i=0;$i -lt $start.physical_line;$i++) { Send-Key $script:InputWindowHandle 0x28 ([uint16[]]@(0x11)) }
    for ($i=0;$i -lt $start.right_count;$i++) { Send-Key $script:InputWindowHandle 0x27 }
    [void](Assert-EditorRecoverySelection $Source $DocumentId $Byte $Byte $Dirty $Undo $Redo ($Description+' exact Source caret'))
    if ($SelectedBytes) {
        $selected=$Source.Substring($start.character_index,$finish.character_index-$start.character_index)
        $steps=[Globalization.StringInfo]::ParseCombiningCharacters($selected).Count
        for ($i=0;$i -lt $steps;$i++) { Send-Key $script:InputWindowHandle 0x27 ([uint16[]]@(0x10)) }
        [void](Assert-EditorRecoverySelection $Source $DocumentId $Byte ($Byte+$SelectedBytes) $Dirty $Undo $Redo ($Description+' forward Source selection'))
    }
    Send-Key $script:InputWindowHandle 0x45 ([uint16[]]@(0x11))
    $live=Assert-EditorRecoverySelection $Source $DocumentId $Byte ($Byte+$SelectedBytes) $Dirty $Undo $Redo ($Description+' same selection in Live') $false
    if ([bool]$live.raw) { throw 'PRODUCT_ASSERTION: normal Source/Live shortcut failed to return Live.' }
    [void](Get-EditorRecoveryCaretDipPoint $live $Description)
    return $live
}

function Invoke-EditorRecoveryBoundaryRoundTrip(
    [string]$Source,[long]$DocumentId,[int]$Byte,[string]$InsertedText,
    [int]$CaretDelta,[bool]$EnterFirst,[int]$Tables,[string]$Name
) {
    $baseline=Switch-ToSource $Source
    if ([bool]$baseline.dirty) { throw 'SETUP_FAILURE: E08 boundary baseline must be saved and clean.' }
    $u=[int]$baseline.undo_count; $r=[int]$baseline.redo_count
    [void](Move-EditorRecoveryBoundaryCaret $Source $DocumentId $Byte 0 $false $u $r 'E08' $Name)
    [void](Save-EditorRecoveryEvidence 'E08' ($Name+' before input'))
    $location=Get-EditorRecoveryBoundaryLocation $Source $Byte
    $eol=if ($Source.Contains("`r`n")) {"`r`n"} else {"`n"}
    if ($EnterFirst) {
        $first=$Source.Insert($location.character_index,$eol)
        $firstHead=$Byte+$eol.Length
        Send-Key $script:InputWindowHandle 0x0D
        $state=Assert-EditorRecoverySelection $first $DocumentId $firstHead $firstHead $true ($u+1) 0 ($Name+' one Enter')
        Assert-EditorRecoveryViewGeometryMetadata $state
        $markers=@($state.views | Where-Object {[int]$_.source_start -eq $firstHead -and [int]$_.source_end -eq $firstHead -and @($_.map) -contains $firstHead})
        if ($markers.Count -eq 0) { throw 'PRODUCT_ASSERTION: E08 Enter before table left no exact source-affine blank marker.' }
        $second=$first.Insert($location.character_index+$eol.Length,$InsertedText)
        $secondHead=$firstHead+$CaretDelta
        Send-Ascii 'x' $script:InputWindowHandle
    } else {
        $first=$Source.Insert($location.character_index,$InsertedText)
        $firstHead=$Byte+$CaretDelta
        Send-Ascii 'x' $script:InputWindowHandle
        $state=Assert-EditorRecoverySelection $first $DocumentId $firstHead $firstHead $true ($u+1) 0 ($Name+' normal x')
        $at=Get-EditorRecoveryBoundaryLocation $first $firstHead
        $second=$first.Insert($at.character_index,$eol)
        $secondHead=$firstHead+$eol.Length
        Send-Key $script:InputWindowHandle 0x0D
    }
    $state=Assert-EditorRecoverySelection $second $DocumentId $secondHead $secondHead $true ($u+2) 0 ($Name+' second normal edit')
    if ([int]$state.tables -ne $Tables) { throw 'PRODUCT_ASSERTION: E08 boundary editing changed table count.' }
    [void](Save-EditorRecoveryEvidence 'E08' ($Name+' after x and Enter'))
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $first $DocumentId $firstHead $firstHead $true ($u+1) 1 ($Name+' Undo second operation'))
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    $restored=Assert-EditorRecoverySelection $Source $DocumentId $Byte $Byte $false $u 2 ($Name+' Undo first operation restores exact source')
    [void](Get-EditorRecoveryCaretDipPoint $restored ($Name+' restored caret'))
    [void](Save-EditorRecoveryEvidence 'E08' ($Name+' Undo restored'))
}

function Invoke-EditorRecoveryE08 {
    $script:CurrentEditorRecoveryCaseId='E08'
    $core=[string]$script:EditorRecoveryExpectedSource
    $state=Switch-ToSource $core
    if ([bool]$state.dirty -or [int]$state.tabs -ne 2 -or [int]$state.tables -ne 0) {
        throw 'SETUP_FAILURE: E08 needs saved Core, no Core table yet, and closed temporary E06/E07 tab.'
    }
    if (-not [bool]$state.ime.open_status.available -or [bool]$state.ime.active) {
        throw 'ENVIRONMENT_BLOCKED: E08 requires observable inactive app-local IME.'
    }
    $script:ImeProfile=$state.active_input_profile
    $script:ImeInitialOpen=[bool]$state.ime.open_status.open
    $script:ImeRestorePending=$true; $script:ImeRestoreAttempted=$false
    $primary=$null
    try {
        if ($script:ImeInitialOpen) {
            Send-Key $script:InputWindowHandle 0x1A
            [void](Wait-ImeOpenState $false $core)
        }
        $id=[long]$state.document_id; $u=[int]$state.undo_count
        [void](Move-EditorRecoveryBoundaryCaret $core $id 0 0 $false $u ([int]$state.redo_count) 'E08' 'empty document source baseline')
        Send-Key $script:InputWindowHandle 0x41 ([uint16[]]@(0x11))
        $length=[Text.Encoding]::UTF8.GetByteCount($core)
        [void](Assert-EditorRecoverySelection $core $id 0 $length $false $u ([int]$state.redo_count) 'E08 full forward selection before emptying')
        # Temporarily clear this owned synthetic document in Source mode: the
        # existing full selection remains intact across this presentation key.
        Send-Key $script:InputWindowHandle 0x45 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $core $id 0 $length $false $u ([int]$state.redo_count) 'E08 selected source remains intact')
        Send-Key $script:InputWindowHandle 0x08
        [void](Assert-EditorRecoverySelection '' $id 0 0 $true ($u+1) 0 'E08 normal Backspace empties synthetic Core')
        Send-Key $script:InputWindowHandle 0x45 ([uint16[]]@(0x11))
        Send-Ascii 'x' $script:InputWindowHandle
        [void](Assert-EditorRecoverySelection 'x' $id 1 1 $true ($u+2) 0 'E08 empty Live accepts x')
        Send-Key $script:InputWindowHandle 0x0D
        [void](Assert-EditorRecoverySelection "x`n" $id 2 2 $true ($u+3) 0 'E08 empty Live accepts Enter once')
        [void](Save-EditorRecoveryEvidence 'E08' 'empty-document-x-enter')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection 'x' $id 1 1 $true ($u+2) 1 'E08 Undo empty Enter')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection '' $id 0 0 $true ($u+1) 2 'E08 Undo empty x')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $core $id 0 $length $false $u 3 'E08 Undo deletion restores source and forward selection')
        Invoke-EditorRecoveryBoundaryRoundTrip $core $id $length 'x' 1 $false 0 'LF EOF'

        # Independent literal; must exactly match the proposed initializer.
        $structured=(@(
            '# Synthetic Editor Recovery fixture','','','alpha beta 日本語','',
            '> quoted line','','- item one','','| first | second |','| --- | --- |',
            '| cell | value |','','','| third | fourth |','| --- | --- |','| next | last |',
            '','~~~text','literal **Markdown**','~~~','','tail sentinel',''
        ) -join "`r`n")
        if ($script:EditorRecoveryWorkspaceSource -cne $structured) {
            throw 'SETUP_FAILURE: E08 requires the reviewed two-table CRLF literal addition before launch.'
        }
        $workspace=Switch-ToSource $structured; $wid=[long]$workspace.document_id
        $cases=@(
            @{p=0; text='x';delta=1;enter=$false;name='CRLF source start'},
            @{p=39;text='x';delta=1;enter=$false;name='consecutive physical blank'},
            @{p=260;text='x';delta=1;enter=$false;name='CRLF EOF'},
            @{p=243;text='x';delta=1;enter=$false;name='outside closing fence'},
            @{p=96;text="x`r`n`r`n";delta=1;enter=$true;name='before first table'},
            @{p=147;text="`r`n`r`nx";delta=5;enter=$false;name='first table end marker'},
            @{p=149;text="`r`nx";delta=3;enter=$false;name='between two tables'},
            @{p=153;text="x`r`n`r`n";delta=1;enter=$true;name='before second table'},
            @{p=205;text='x';delta=1;enter=$false;name='blank after second table'}
        )
        foreach ($case in $cases) {
            Invoke-EditorRecoveryBoundaryRoundTrip $structured $wid $case.p $case.text $case.delta $case.enter 2 $case.name
        }
        [void](Switch-ToSource $core)
        Set-EditorRecoveryCase 'E08' 'NOT_RUN' @{
            verified=@('temporary empty Live document x/Enter and directed Undo restoration','LF EOF',($cases | ForEach-Object {$_.name}))
            fixed_CRLF_bytes=260; table_count=2; source_restored=$true
            not_run=@('actual screenshot review: doubled whitespace, overlap, caret flight and fence context')
            reason='Normal input and fixed source/history oracles execute here; visual boundary acceptance needs review of the retained actual images.'
        }
    } catch { $primary=$_.Exception; throw }
    finally {
        try { Restore-ImeOpenState }
        catch { if ($null -eq $primary) {throw}; $script:Report.ime_open_restoration_error=$_.Exception.Message }
    }
}

function Invoke-EditorRecoveryJapaneseCell($Baseline,[string]$ExpectedSource,[int]$ExpectedHead,[string]$Phase) {
    $profile=$Baseline.active_input_profile
    if ($null -eq $profile -or $profile.status -cne 'active' -or
        $profile.profile_type -cne 'text_service' -or $profile.language_id -cne '0x0411' -or
        -not [bool]$Baseline.ime.open_status.available -or [bool]$Baseline.ime.active) {
        throw 'ENVIRONMENT_BLOCKED: E10 cell requires an observable installed active Japanese text service.'
    }
    $parent=Get-UiParentRoot $Baseline
    $editor=Convert-DiagnosticWindowHandle $parent.editor_hwnd
    $main=Convert-DiagnosticWindowHandle $parent.hwnd
    $compact=Convert-DiagnosticWindowHandle $parent.compact_hwnd
    Assert-EditorRecoveryMainIdentity $script:WindowHandle $main $editor ($Phase+' before composition')
    $focus=[MdliteRustAcceptanceNative]::ObserveOwnedEditorFocus($editor,$main,$compact,$script:ProcessId)
    if (-not $focus.registered_editor_window -or -not $focus.gui_thread_info_available) { throw 'OBSERVER_FAILURE: E10 native editor focus/caret unavailable.' }
    if (-not $focus.foreground_is_expected) { throw 'ENVIRONMENT_BLOCKED: E10 exact owned editor lost foreground before cell input.' }
    if (-not $focus.focus_is_expected -or -not $focus.caret_is_expected) { throw 'PRODUCT_ASSERTION: E10 cell input owner is not the registered native editor focus/caret.' }
    $script:ImeProfile=$profile; $script:ImeInitialOpen=[bool]$Baseline.ime.open_status.open
    $script:ImeRestorePending=$true; $script:ImeRestoreAttempted=$false
    $primary=$null; $script:ImeEscapesSentForComposition=0
    try {
        if (-not $script:ImeInitialOpen) {
            Send-Key $script:InputWindowHandle 0x16
            [void](Wait-ImeOpenState $true ([string]$Baseline.source))
        }
        Send-Ascii 'nihongo' $script:InputWindowHandle
        $reading=Wait-EditorRecoveryStableState -WaitMs 10000 -Description ($Phase+' complete raw reading') -Condition {
            param($s)
            Assert-EditorRecoveryImeInvariant $s $Baseline ($Phase+' reading')
            [long]$s.document_id -eq [long]$Baseline.document_id -and [bool]$s.ime.active -and $s.ime.preedit -ceq 'にほんご'
        }
        [void](Save-EditorRecoveryEvidence 'E10' ($Phase+' reading'))
        Send-Key $script:InputWindowHandle 0x20
        $converted=Wait-EditorRecoveryStableState -WaitMs 10000 -Description ($Phase+' Space conversion') -Condition {
            param($s)
            Assert-EditorRecoveryImeInvariant $s $Baseline ($Phase+' conversion')
            [long]$s.document_id -eq [long]$Baseline.document_id -and [bool]$s.ime.active -and
                -not [string]::IsNullOrEmpty([string]$s.ime.preedit) -and $s.ime.preedit -cne 'にほんご'
        }
        for ($candidate=0;$candidate -lt 5 -and $converted.ime.preedit -cne '日本語';$candidate++) {
            [void](Save-EditorRecoveryEvidence 'E10' ($Phase+' candidate-'+$candidate))
            $previousCandidatePreedit=[string]$converted.ime.preedit
            Send-Key $script:InputWindowHandle 0x20
            $converted=Wait-EditorRecoveryStableState -WaitMs 5000 -Description ($Phase+' distinct stable candidate after bounded normal Space') -Condition {
                param($s)
                Assert-EditorRecoveryImeInvariant $s $Baseline ($Phase+' candidate')
                [long]$s.document_id -eq [long]$Baseline.document_id -and [bool]$s.ime.active -and
                    -not [string]::IsNullOrEmpty([string]$s.ime.preedit) -and
                    [string]$s.ime.preedit -cne $previousCandidatePreedit -and
                    (Test-EditorRecoveryJapaneseText ([string]$s.ime.preedit))
            }
        }
        if ($converted.ime.preedit -cne '日本語') {
            throw 'ENVIRONMENT_BLOCKED: fixed 日本語 candidate not observed within conversion Space plus five distinct normal Space candidate steps; no substitute text committed.'
        }
        [void](Save-EditorRecoveryEvidence 'E10' ($Phase+' exact-Japanese-candidate'))
        Send-Key $script:InputWindowHandle 0x0D
        $committed=Wait-EditorRecoveryStableState -WaitMs 10000 -Description ($Phase+' one native IME commit') -Condition {
            param($s)
            (Test-SameInputProfile $s) -and [long]$s.document_id -eq [long]$Baseline.document_id -and
                -not [bool]$s.ime.active -and [string]::IsNullOrEmpty([string]$s.ime.preedit) -and
                $s.source -ceq $ExpectedSource -and [int]$s.selection.anchor -eq $ExpectedHead -and
                [int]$s.selection.head -eq $ExpectedHead -and [bool]$s.dirty -and
                [int]$s.undo_count -eq ([int]$Baseline.undo_count+1) -and [int]$s.redo_count -eq 0
        }
        [void](Save-EditorRecoveryEvidence 'E10' ($Phase+' committed'))
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection ([string]$Baseline.source) ([long]$Baseline.document_id) ([int]$Baseline.selection.anchor) ([int]$Baseline.selection.head) ([bool]$Baseline.dirty) ([int]$Baseline.undo_count) 1 ($Phase+' one Undo restores exact source and destination selection'))
        Send-Key $script:InputWindowHandle 0x59 ([uint16[]]@(0x11))
        return Assert-EditorRecoverySelection $ExpectedSource ([long]$Baseline.document_id) $ExpectedHead $ExpectedHead $true ([int]$Baseline.undo_count+1) 0 ($Phase+' Redo restores one Japanese cell commit')
    } catch { $primary=$_.Exception; throw }
    finally {
        try { Restore-ImeOpenState }
        catch { if ($null -eq $primary) {throw}; $script:Report.ime_open_restoration_error=$_.Exception.Message }
    }
}

function Invoke-EditorRecoveryE10 {
    $script:CurrentEditorRecoveryCaseId='E10'
    $core=[string]$script:EditorRecoveryExpectedSource
    $baseline=Switch-ToSource $core
    if ([int]$baseline.tabs -ne 2 -or [bool]$baseline.dirty) { throw 'SETUP_FAILURE: E10 must follow E09 save with the original two tabs.' }
    $initialProfile=Get-EditorRecoveryProfileIdentity $baseline.active_input_profile
    $initialOpen=[bool]$baseline.ime.open_status.open
    if (-not [bool]$baseline.ime.open_status.available -or [bool]$baseline.ime.active) { throw 'ENVIRONMENT_BLOCKED: E10 initial IME state unavailable or already composing.' }
    $script:ImeProfile=$baseline.active_input_profile
    if ($initialOpen) { Send-Key $script:InputWindowHandle 0x1A; [void](Wait-ImeOpenState $false $core) }
    $id=[long]$baseline.document_id; $u=[int]$baseline.undo_count
    $end=[Text.Encoding]::UTF8.GetByteCount($core)
    [void](Move-EditorRecoveryBoundaryCaret $core $id $end 0 $false $u ([int]$baseline.redo_count) 'E10' 'normal table authoring at LF EOF')
    Send-Key $script:InputWindowHandle 0x0D; $source=$core+"`n"; $u++
    [void](Assert-EditorRecoverySelection $source $id ($end+1) ($end+1) $true $u 0 'E10 first paragraph gap Enter')
    Send-Key $script:InputWindowHandle 0x0D; $prefix=$core+"`n`n"; $u++; $p=$end+2
    [void](Assert-EditorRecoverySelection $prefix $id $p $p $true $u 0 'E10 second paragraph gap Enter')
    Send-Ascii '| H | J |' $script:InputWindowHandle; $source=$prefix+'| H | J |'; $u++
    [void](Assert-EditorRecoverySelection $source $id ($p+9) ($p+9) $true $u 0 'E10 normal VK table header notation')
    Send-Key $script:InputWindowHandle 0x0D; $source+="`n"; $u++
    [void](Assert-EditorRecoverySelection $source $id ($p+10) ($p+10) $true $u 0 'E10 normal Enter after header')
    Send-Ascii '| --- | --- |' $script:InputWindowHandle; $source+='| --- | --- |'; $u++
    [void](Assert-EditorRecoverySelection $source $id ($p+23) ($p+23) $true $u 0 'E10 delimiter creates real Markdown table')
    $t0="| H | J |`n| --- | --- |`n|  |  |"
    $t1="| H | J |`n| --- | --- |`n| 日本語 |  |"
    $t2="| H | J |`n| --- | --- |`n| 日本語 | 日本語 |"
    $t3="| H | 日本語 |`n| --- | --- |`n| 日本語 | 日本語 |"
    $t4="| 日本語 | 日本語 |`n| --- | --- |`n| 日本語 | 日本語 |"
    Send-Key $script:InputWindowHandle 0x0D; $u++
    $state=Assert-EditorRecoverySelection ($prefix+$t0) $id ($p+26) ($p+26) $true $u 0 'E10 Enter creates first empty data row and selects cell0'
    if ([int]$state.tables -ne 1 -or [bool]$state.raw) { throw 'PRODUCT_ASSERTION: typed notation failed to create the single Live table.' }
    [void](Save-EditorRecoveryEvidence 'E10' 'typed-table-created')
    $active=$state.active_input_profile
    if ($null -eq $active -or $active.status -cne 'active' -or $active.profile_type -cne 'text_service' -or $active.language_id -cne '0x0411') {
        $state=Request-EditorRecoveryProfile 'E10-table' 'ATOK|Microsoft'
        if ($null -eq $state) { throw 'ENVIRONMENT_BLOCKED: no unambiguous installed Japanese profile for real table-cell composition.' }
    }
    $state=Assert-EditorRecoverySelection ($prefix+$t0) $id ($p+26) ($p+26) $true $u 0 'E10 profile selection preserves the exact table destination'
    # All four cells use independent literal 日本語 oracles, never a captured
    # arbitrary candidate as an expected replacement or Unicode substitution.
    $state=Invoke-EditorRecoveryJapaneseCell $state ($prefix+$t1) ($p+35) 'data-cell0'; $u++
    Send-Key $script:InputWindowHandle 0x09
    $state=Assert-EditorRecoverySelection ($prefix+$t1) $id ($p+38) ($p+38) $true $u 0 'E10 Tab selects empty data-cell1'
    $state=Invoke-EditorRecoveryJapaneseCell $state ($prefix+$t2) ($p+47) 'data-cell1'; $u++
    Send-Key $script:InputWindowHandle 0x0D ([uint16[]]@(0x10))
    $state=Assert-EditorRecoverySelection ($prefix+$t2) $id ($p+6) ($p+7) $true $u 0 'E10 Shift+Enter selects same-column header-cell1'
    $state=Invoke-EditorRecoveryJapaneseCell $state ($prefix+$t3) ($p+15) 'header-cell1'; $u++
    Send-Key $script:InputWindowHandle 0x09 ([uint16[]]@(0x10))
    $state=Assert-EditorRecoverySelection ($prefix+$t3) $id ($p+2) ($p+3) $true $u 0 'E10 Shift+Tab selects full header-cell0'
    $state=Invoke-EditorRecoveryJapaneseCell $state ($prefix+$t4) ($p+11) 'header-cell0'; $u++
    $source=$prefix+$t4
    Send-Key $script:InputWindowHandle 0x24
    [void](Assert-EditorRecoverySelection $source $id ($p+2) ($p+2) $true $u 0 'E10 Home at header-cell0 start')
    Send-Key $script:InputWindowHandle 0x28
    [void](Assert-EditorRecoverySelection $source $id ($p+42) ($p+42) $true $u 0 'E10 Down to data-cell0 at shared left geometry')
    Send-Key $script:InputWindowHandle 0x26
    [void](Assert-EditorRecoverySelection $source $id ($p+2) ($p+2) $true $u 0 'E10 Up returns header-cell0 start')
    Send-Key $script:InputWindowHandle 0x23
    Send-Key $script:InputWindowHandle 0x27
    [void](Assert-EditorRecoverySelection $source $id ($p+14) ($p+14) $true $u 0 'E10 Right at cell end enters next cell without structural pipe')
    Send-Key $script:InputWindowHandle 0x25
    [void](Assert-EditorRecoverySelection $source $id ($p+11) ($p+11) $true $u 0 'E10 Left at next-cell start returns previous cell end')
    Send-Key $script:InputWindowHandle 0x0D
    [void](Assert-EditorRecoverySelection $source $id ($p+42) ($p+51) $true $u 0 'E10 Enter selects same-column full data cell')
    Send-Key $script:InputWindowHandle 0x23
    [void](Assert-EditorRecoverySelection $source $id ($p+51) ($p+51) $true $u 0 'E10 End inside Japanese data-cell0')
    $script:ImeProfile=(Read-Diagnostic).active_input_profile
    if ([bool](Read-Diagnostic).ime.open_status.open) { Send-Key $script:InputWindowHandle 0x1A; [void](Wait-ImeOpenState $false $source) }
    Send-Ascii '|tail' $script:InputWindowHandle
    $pipe="| 日本語 | 日本語 |`n| --- | --- |`n| 日本語\|tail | 日本語 |"
    $source=$prefix+$pipe; $u++
    [void](Assert-EditorRecoverySelection $source $id ($p+57) ($p+57) $true $u 0 'E10 normal pipe is escaped in source and remains meaningful cell text')
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection ($prefix+$t4) $id ($p+51) ($p+51) $true ($u-1) 1 'E10 one Undo removes pipe-tail operation only')
    Send-Key $script:InputWindowHandle 0x59 ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $source $id ($p+57) ($p+57) $true $u 0 'E10 Redo restores escaped raw pipe')
    [void](Save-EditorRecoveryEvidence 'E10' 'four-Japanese-cells-and-escaped-pipe')

    Invoke-UiPaletteCommand '行を下に追加'; $row=$source+"`n|  |  |"; $u++
    [void](Assert-EditorRecoverySelection $row $id ($p+79) ($p+79) $true $u 0 'E10 palette adds blank row below active data row')
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $source $id ($p+57) ($p+57) $true ($u-1) 1 'E10 one Undo row addition preserves pipe')
    Send-Key $script:InputWindowHandle 0x59 ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $row $id ($p+79) ($p+79) $true $u 0 'E10 Redo row addition')
    [void](Move-EditorRecoveryBoundaryCaret $row $id ($p+74) 0 $true $u 0 'E10' 'new blank row deletion target')
    Invoke-UiPaletteCommand '行を削除（見出し・最後の行を保護）'; $u++
    [void](Assert-EditorRecoverySelection $source $id ($p+71) ($p+71) $true $u 0 'E10 palette deletes new row only')
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $row $id ($p+74) ($p+74) $true ($u-1) 1 'E10 one Undo row deletion restores targeted caret')
    Send-Key $script:InputWindowHandle 0x59 ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $source $id ($p+71) ($p+71) $true $u 0 'E10 Redo deletes new row')
    [void](Move-EditorRecoveryBoundaryCaret $source $id ($p+42) 0 $true $u 0 'E10' 'column-right target data-cell0')
    $column=$prefix+"| 日本語 |  | 日本語 |`n| --- | --- | --- |`n| 日本語\|tail |  | 日本語 |"
    Invoke-UiPaletteCommand '列を右に追加'; $u++
    $columnEnd=[Text.Encoding]::UTF8.GetByteCount($column)
    [void](Assert-EditorRecoverySelection $column $id $columnEnd $columnEnd $true $u 0 'E10 palette inserts blank column1')
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $source $id ($p+42) ($p+42) $true ($u-1) 1 'E10 one Undo column addition preserves original pipes')
    Send-Key $script:InputWindowHandle 0x59 ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $column $id $columnEnd $columnEnd $true $u 0 'E10 Redo column addition')
    [void](Move-EditorRecoveryBoundaryCaret $column $id ($p+14) 0 $true $u 0 'E10' 'inserted empty header column1 deletion target')
    Invoke-UiPaletteCommand '列を削除（最後の列を保護）'; $u++
    [void](Assert-EditorRecoverySelection $source $id ($p+71) ($p+71) $true $u 0 'E10 palette deletes inserted column1 only')
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $column $id ($p+14) ($p+14) $true ($u-1) 1 'E10 one Undo column deletion')
    Send-Key $script:InputWindowHandle 0x59 ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $source $id ($p+71) ($p+71) $true $u 0 'E10 Redo column deletion preserves escaped pipe')
    [void](Move-EditorRecoveryBoundaryCaret $source $id ($p+57) 0 $true $u 0 'E10' 'table EOF-row Enter target')
    Send-Key $script:InputWindowHandle 0x0D; $u++
    [void](Assert-EditorRecoverySelection $row $id ($p+74) ($p+74) $true $u 0 'E10 Enter at final data row appends one blank row and selects same-column cell')
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $source $id ($p+57) ($p+57) $true ($u-1) 1 'E10 one Undo final-row Enter'); $u--
    Send-Key $script:InputWindowHandle 0x0D ([uint16[]]@(0x11)); $exit=$source+"`n`n"; $u++
    [void](Assert-EditorRecoverySelection $exit $id ($p+73) ($p+73) $true $u 0 'E10 Ctrl+Enter exits to a real paragraph after table')
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $source $id ($p+57) ($p+57) $true ($u-1) 1 'E10 one Undo table exit')
    Send-Key $script:InputWindowHandle 0x59 ([uint16[]]@(0x11))
    [void](Assert-EditorRecoverySelection $exit $id ($p+73) ($p+73) $true $u 0 'E10 Redo table exit')
    Send-Ascii 'z' $script:InputWindowHandle; $u++
    [void](Assert-EditorRecoverySelection ($exit+'z') $id ($p+74) ($p+74) $true $u 0 'E10 normal letter after table exit')
    Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11)); $u--
    [void](Assert-EditorRecoverySelection $exit $id ($p+73) ($p+73) $true $u 1 'E10 Undo letter preserves complete table')
    $geometry=Read-Diagnostic
    Assert-EditorRecoveryViewGeometryMetadata $geometry -RequireTables
    $tables=@($geometry.table_geometry)
    if ($tables.Count -ne 1 -or [int]$tables[0].source_start -ne $p -or [int]$tables[0].source_end -ne ($p+71)) {
        throw 'PRODUCT_ASSERTION: E10 table source geometry no longer covers the exact pipe-preserving literal.'
    }
    foreach ($field in @('width','height')) {
        $metric=[double]$tables[0].$field
        if ([double]::IsNaN($metric) -or [double]::IsInfinity($metric) -or $metric -le 0) { throw 'PRODUCT_ASSERTION: E10 table geometry is nonfinite or nonpositive.' }
    }
    foreach ($range in @(@(2,11),@(14,23),@(42,57),@(60,69))) {
        $cells=@($geometry.views | Where-Object {$null -ne $_.table -and [int]$_.source_start -eq ($p+$range[0]) -and [int]$_.source_end -eq ($p+$range[1])})
        if ($cells.Count -ne 1) { throw 'PRODUCT_ASSERTION: E10 missing exact mapped Japanese cell geometry.' }
        foreach ($field in @('width','height')) {
            $metric=[double]$cells[0].$field
            if ([double]::IsNaN($metric) -or [double]::IsInfinity($metric) -or $metric -le 0) {
                throw 'PRODUCT_ASSERTION: E10 nonfinite/nonpositive mapped Japanese cell geometry.'
            }
        }
    }
    [void](Save-EditorRecoveryEvidence 'E10' 'table-mutations-and-Undo-complete')
    Save-ActiveFixture $exit $script:EditorRecoveryBlankPath
    $script:EditorRecoveryExpectedSource=$exit
    if (-not (Test-EditorRecoveryProfileIdentity $initialProfile (Read-Diagnostic).active_input_profile)) {
        $restored=Request-EditorRecoveryProfile 'E10-restore' $null $initialProfile
        if ($null -eq $restored) { throw 'ENVIRONMENT_BLOCKED: initial profile restoration after E10 failed.' }
    }
    $script:ImeProfile=(Read-Diagnostic).active_input_profile
    $script:ImeInitialOpen=$initialOpen; $script:ImeRestorePending=$true; $script:ImeRestoreAttempted=$false
    Restore-ImeOpenState
    [void](Assert-EditorRecoverySelection $exit $id ($p+73) ($p+73) $false $u 1 'E10 saved table/source/checkpoint before E11 and E13')
    Set-EditorRecoveryCase 'E10' 'NOT_RUN' @{
        verified=@('normal VK header/delimiter/data-row creation','four real IME 日本語 commits with exact one-Undo/Redo',
            'Tab/Shift+Tab, normal arrows, Enter/Shift+Enter and Ctrl+Enter exit','meaningful raw escaped pipe and one-Undo/Redo',
            'normal palette row/column addition/deletion, each with one-Undo/Redo','ordinary letter after table and Undo','normal save checkpoint')
        not_run=@('cell width/height/overlap and horizontal-scroll actual screenshot review','explicit horizontal-scroll gesture on a wide table')
        reason='Listed normal-input source/history steps execute here; geometry image review and wide-table horizontal scrolling remain unverified.'
    }
}

function Invoke-EditorRecoverySuite {
    if ($UiParity -or $StartupSmoke) { throw 'Use only one GUI acceptance mode per run.' }
    if ([string]::IsNullOrWhiteSpace($ExpectedExecutableSha256) -or $ExpectedExecutableSha256 -notmatch '^[0-9a-fA-F]{64}$') {
        throw 'EditorRecovery requires -ExpectedExecutableSha256 from the selected Release build.'
    }
    if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf) -or $ExecutablePath -notmatch '[\\/]target[\\/]release[\\/]mdlite\.exe$') {
        throw 'EditorRecovery accepts only target/release/mdlite.exe.'
    }
    $runId = '{0}-{1}' -f [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ'),[guid]::NewGuid().ToString('N')
    $script:UiRunDirectory = Join-Path $RunRoot $runId
    [void][IO.Directory]::CreateDirectory($script:UiRunDirectory)
    $script:ReportPath = Join-Path $script:UiRunDirectory 'result.json'
    $script:DiagnosticPath = Join-Path $script:UiRunDirectory 'diagnostic.json'
    $script:ForegroundLogPath = Join-Path $script:UiRunDirectory 'foreground.log'
    $caseIds = @('E01','E02','E03','E04','E05','E06','E07','E08','E09','E10','E11','E12','E13','E14','U01','U02','U03')
    $notRun = @{
        E06='Wrap/reverse visual review and other pointer cases remain untested.'
        E07='Empty task continuation and the complete Enter/Tab/Shift+Tab row remain untested.'
        E08='Block boundaries, multiple tables, and fence-adjacent insertion remain untested.'
        E10='Table movement, row/column mutation, and table Undo remain untested.'
        E12='Resize, DPI, theme/font, minimize/restore, and viewport geometry remain untested.'
        E14='Save failure, external changes, missing recovery, and close during composition need fault fixtures.'
        U01='Owner-draw labels and enabled/disabled/hover/focus states need a separate visual case.'
        U02='Calendar width, markers, month navigation, and daily-open behavior need a separate visual case.'
        U03='Git/Outline/status/search rendering and Esc routing need a separate visual case.'
    }
    $script:Report.status = 'RUNNING'
    $script:Report.configuration = 'Release'
    $script:Report.executable_path = [IO.Path]::GetFullPath($ExecutablePath)
    $script:Report.harness_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:Report.foreground_log_path = $script:ForegroundLogPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
    $script:Report.input_delivery = 'Document ASCII uses guarded current-layout native virtual-key SendInput; IME uses selected installed profiles. Unicode command-palette query packets and owned SaveAs WM_SETTEXT/GETTEXT are fixture setup only, not document or IME keyboard proof. OS clipboard is not used.'
    $script:Report.normal_close = @()
    $editorRecoveryReportFields = [ordered]@{
        suite='EditorRecovery'; run_id=$runId
        expected_executable_sha256=$ExpectedExecutableSha256.ToLowerInvariant()
        harness_sha256_after=$null
        process_image_path=$null; process_image_sha256=$null; process_identity_matches_expected=$false
        process_identities=@(); appdata_isolated=$true
        appdata_isolation_scope='Owned LOCALAPPDATA and APPDATA under this run; host AppData is not read or changed.'
        workspace_root=$null; appdata_root=$null
        active_input_profile_initial=$null; profile_requests=@(); ime_profile_restoration=$null
        save_as_dialog_evidence=@()
        ime_profile_description=$null; ime_profile_type=$null; ime_profile_language=$null
        ime_committed_suffix=$null; ime_composition_preedit=$null; ime_conversion_candidate=$null
        initial_app_local_ime_open_status=$null; ascii_input_profile_current=$null; ascii_keyboard_layout_transitions=@()
        profile_activation_scope='TF_IPPMF_FORSESSION applies the selected profile to all threads on the current desktop session; no enable/disable/default-profile flags are used.'
        other_app_profile_state='NOT_VERIFIED: only the owned MDLite thread profile and app-local open state are read back/restored.'
        evidence=@(); cleanup_note=$null; ended_utc=$null
        failure_exception_type=$null; failure_exception_message=$null
        failure_script_stack_trace=$null; failure_body_diagnostic_stage=$null
        editor_recovery_cases=@(foreach ($id in $caseIds) {
            [pscustomobject]@{
                id=$id; status='NOT_RUN'
                reason=if ($notRun.ContainsKey($id)) { $notRun[$id] } else { 'Automated assertion did not reach this case.' }
                details=$null; evidence=@(); visual_review_status='NOT_RUN'
            }
        })
    }
    foreach ($field in $editorRecoveryReportFields.GetEnumerator()) {
        $script:Report.Add($field.Key, $field.Value)
    }
    $script:EditorRecoveryEvidenceOrdinal=0; $script:EditorRecoveryCurrentCaseId=$null
    $script:Report.git_head=(& git -C $ProjectRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not pin current Git HEAD.' }
    $script:Report.executable_sha256_before=(Get-FileHash -LiteralPath $ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($script:Report.executable_sha256_before -cne $ExpectedExecutableSha256.ToLowerInvariant()) { throw 'Release executable hash does not match the supplied hash.' }
    $script:Report.source_tree_sha256_before=Get-SourceTreeSha256
    New-EditorRecoveryFixture

    try {
        $script:CurrentEditorRecoveryCaseId='E01'
        $script:UiFixtureRoot=$script:EditorRecoveryNoWorkspaceRoot
        [string[]]$noPaths=@()
        Start-UiParityProcess $noPaths $script:DiagnosticPath
        $initial=Wait-Diagnostic {
            param($s)
            $s.source -ceq '' -and [bool]$s.dirty -and [int]$s.tabs -eq 1 -and
            [int]$s.revision -eq 0 -and [int]$s.selection.anchor -eq 0 -and
            [int]$s.selection.head -eq 0 -and [int]$s.undo_count -eq 0 -and
            [int]$s.redo_count -eq 0 -and -not [bool]$s.ime.active -and
            [string]::IsNullOrEmpty([string]$s.ime.preedit)
        } 'E01 fresh zero-argument unsaved startup snapshot'
        $script:Report.active_input_profile_initial=Get-EditorRecoveryProfileIdentity $initial.active_input_profile
        $script:Report.ascii_input_profile_current=$script:Report.active_input_profile_initial
        $script:Report.initial_app_local_ime_open_status=$initial.ime.open_status
        [void](Save-EditorRecoveryEvidence 'E01' 'no-workspace-start')
        $priorId=[long]$initial.document_id
        if ($initial.source -cne '' -or -not [bool]$initial.dirty -or
            [int]$initial.selection.anchor -ne 0 -or [int]$initial.selection.head -ne 0 -or
            [int]$initial.undo_count -ne 0 -or [int]$initial.redo_count -ne 0 -or [int]$initial.tabs -ne 1) {
            throw 'PRODUCT_ASSERTION: E01 zero-argument startup did not expose one empty unsaved document at offset zero with fresh history.'
        }
        $noWorkspaceStartupPath=Join-Path $script:EditorRecoveryNoWorkspaceRoot 'startup-unsaved-document.md'
        Save-UnsavedEditorRecoveryFixture '' $noWorkspaceStartupPath
        $startupSaved=Wait-Diagnostic {
            param($s)
            [long]$s.document_id -eq $priorId -and $s.source -ceq '' -and
            -not [bool]$s.dirty -and [int]$s.selection.anchor -eq 0 -and
            [int]$s.selection.head -eq 0 -and [int]$s.undo_count -eq [int]$initial.undo_count -and
            [int]$s.redo_count -eq [int]$initial.redo_count -and [int]$s.tabs -eq 1
        } 'clean saved zero-argument startup document'
        $script:Report.fixtures += [pscustomobject]@{
            path=$noWorkspaceStartupPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
            role='zero-argument no-Workspace startup document saved to an owned synthetic path before Ctrl+N'
            saved_sha256=(Get-FileHash -LiteralPath $noWorkspaceStartupPath -Algorithm SHA256).Hash.ToLowerInvariant()
            saved_bytes=[IO.File]::ReadAllBytes($noWorkspaceStartupPath).Length
        }
        Send-Key $script:InputWindowHandle 0x4E ([uint16[]]@(0x11))
        $newDocument=Wait-Diagnostic { param($s) [long]$s.document_id -ne $priorId -and [int]$s.tabs -eq 2 } 'new blank document without Workspace'
        if ($newDocument.source -cne '' -or -not [bool]$newDocument.dirty -or
            [int]$newDocument.selection.anchor -ne 0 -or [int]$newDocument.selection.head -ne 0 -or
            [int]$newDocument.undo_count -ne 0 -or [int]$newDocument.redo_count -ne 0) {
            throw 'PRODUCT_ASSERTION: E01 new unsaved document did not start at empty source, offset zero, and its initial dirty/history state.'
        }
        [void](Save-EditorRecoveryEvidence 'E01' 'new-document-initial-unsaved-state')
        [void](Invoke-EditorViewportBodyClick 'E01' 'no-workspace-new-document-before-a')
        $inputSentUtc = [DateTime]::UtcNow
        Send-Ascii 'a' $script:InputWindowHandle
        [void](Assert-EditorRecoveryDocument 'a' 1 $true 'E01 no-Workspace ASCII input')
        $script:Report.native_editor_focus_checks[-1].input_observed_ms =
            [Math]::Round(([DateTime]::UtcNow - $inputSentUtc).TotalMilliseconds, 1)
        [void](Save-EditorRecoveryEvidence 'E01' 'no-workspace-input')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        $afterUndo=Wait-Diagnostic {
            param($s)
            [long]$s.document_id -eq [long]$newDocument.document_id -and
            $s.source -ceq '' -and [bool]$s.dirty -and
            [int]$s.selection.anchor -eq 0 -and [int]$s.selection.head -eq 0 -and
            [int]$s.undo_count -eq [int]$newDocument.undo_count -and
            [int]$s.redo_count -gt [int]$newDocument.redo_count
        } 'E01 Undo restores the empty source and original unsaved state'
        Add-Observation 'E01 Undo restores empty source while retaining the untitled dirty state' $afterUndo
        [void](Save-EditorRecoveryEvidence 'E01' 'undo-restored-initial-unsaved-state')
        $noWorkspaceSavedPath=Join-Path $script:EditorRecoveryNoWorkspaceRoot 'new-unsaved-document.md'
        Save-UnsavedEditorRecoveryFixture '' $noWorkspaceSavedPath
        [void](Wait-Diagnostic {
            param($s)
            [long]$s.document_id -eq [long]$newDocument.document_id -and $s.source -ceq '' -and
            -not [bool]$s.dirty -and [int]$s.selection.anchor -eq 0 -and
            [int]$s.selection.head -eq 0 -and [int]$s.undo_count -eq [int]$afterUndo.undo_count -and
            [int]$s.redo_count -eq [int]$afterUndo.redo_count -and [int]$s.tabs -eq 2
        } 'clean saved Ctrl+N document before normal close')
        $script:Report.fixtures += [pscustomobject]@{
            path=$noWorkspaceSavedPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
            role='new no-Workspace document saved to a harness-owned synthetic path'
            saved_sha256=(Get-FileHash -LiteralPath $noWorkspaceSavedPath -Algorithm SHA256).Hash.ToLowerInvariant()
            saved_bytes=[IO.File]::ReadAllBytes($noWorkspaceSavedPath).Length
        }
        Close-EditorRecoveryProcess 'no-workspace'
        Set-EditorRecoveryCase 'E01' 'PASS' @{
            no_workspace_start='PASS'; new_document_input='PASS'
            startup_document_saved_to_owned_path='PASS'
            startup_document_state=@{document_id=$priorId;source='';dirty=[bool]$initial.dirty;undo_count=[int]$initial.undo_count;redo_count=[int]$initial.redo_count}
            startup_document_saved=@{document_id=[long]$startupSaved.document_id;source=[string]$startupSaved.source;dirty=[bool]$startupSaved.dirty}
            new_document_initial_state=@{document_id=[long]$newDocument.document_id;source='';dirty=$true;undo_count=0;redo_count=0}
            native_editor_focus_before_after=@($script:Report.native_editor_focus_checks | Where-Object {$_.case_id -ceq 'E01'})
            first_input_observed_ms=$script:Report.native_editor_focus_checks[-1].input_observed_ms
            undo_restore=@{source='';dirty=[bool]$afterUndo.dirty;undo_count=[int]$afterUndo.undo_count;redo_count=[int]$afterUndo.redo_count}
            saved_to_owned_synthetic_path='PASS'
            save_as_dialog_evidence=@($script:Report.save_as_dialog_evidence)
        }

        $script:UiFixtureRoot=Join-Path $script:UiRunDirectory 'workspace'
        $ownedWorkspaceRoot=[IO.Path]::GetFullPath((Join-Path $script:UiRunDirectory 'workspace'))
        if (-not [string]::Equals([IO.Path]::GetFullPath($script:UiFixtureRoot),$ownedWorkspaceRoot,[StringComparison]::OrdinalIgnoreCase)) {
            throw 'SETUP_FAILURE: initial Workspace root is not the exact run-owned fixture root.'
        }
        $workspaceStatePath=Join-Path $script:UiFixtureRoot '.mdlite\.state'
        if (Test-Path -LiteralPath $workspaceStatePath) {
            $workspaceStateItem=Get-Item -LiteralPath $workspaceStatePath -ErrorAction Stop
            if (-not $workspaceStateItem.PSIsContainer -or
                @(Get-ChildItem -LiteralPath $workspaceStatePath -Force -ErrorAction Stop).Count -ne 0) {
                throw 'SETUP_FAILURE: initial Workspace .mdlite/.state must be absent or empty; existing state was preserved.'
            }
        }
        Start-UiParityProcess @($script:UiFixtureRoot,$script:EditorRecoveryWorkspacePath,$script:EditorRecoveryBlankPath,$script:EditorRecoveryPointerPath) $script:DiagnosticPath
        [void](Wait-Diagnostic { param($s) [int]$s.tabs -eq 3 } 'three owned Workspace fixtures including byte-exact editor.md')
        $script:Report.workspace_settings['workspace_root_readbacks']['initial'] =
            Get-EditorRecoveryWorkspaceRootReceipt $script:UiFixtureRoot
        [void](Save-EditorRecoveryEvidence 'E01' 'workspace-open')
        $e01=@($script:Report.editor_recovery_cases | Where-Object {$_.id -eq 'E01'})[0]
        $e01.details.workspace_opened='PASS'

        $script:CurrentEditorRecoveryCaseId='E02'
        [void](Switch-ToSource '')
        [void](Save-EditorRecoveryEvidence 'E02' 'empty-start')
        [void](Assert-EditorRecoveryWorkspaceAutoSaveDisabledSetup $script:Report.workspace_settings)
        [void](Invoke-EditorViewportBodyClick 'E02' 'workspace-core-input-before-alpha-beta')
        $source=''
        $inputSentUtc = [DateTime]::UtcNow
        Send-Ascii 'alpha beta' $script:InputWindowHandle
        $source='alpha beta'
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 ASCII source')
        $script:Report.native_editor_focus_checks[-1].input_observed_ms =
            [Math]::Round(([DateTime]::UtcNow - $inputSentUtc).TotalMilliseconds, 1)
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 paragraph Enter')
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 blank-line Enter')
        Send-Ascii '# Heading' $script:InputWindowHandle
        $source += '# Heading'
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 heading source')
        [void](Save-EditorRecoveryEvidence 'E02' 'heading-visible')
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf
        Send-Ascii '**bold**' $script:InputWindowHandle
        $source += '**bold**'
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 bold source')
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf
        Send-Ascii '> quote' $script:InputWindowHandle
        $source += '> quote'
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 quote source')
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf + '> '
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 quote Enter continues prefix')
        [void](Save-EditorRecoveryEvidence 'E02' 'quote-continuation')
        Send-Ascii 'quoted next' $script:InputWindowHandle
        $source += 'quoted next'
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 quoted continuation text')
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf + '> '
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 second quote prefix')
        Send-Key $script:InputWindowHandle 0x0D
        $source = $source.Substring(0, $source.Length - 2)
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 empty quote Enter exits prefix')
        Send-Ascii '- item' $script:InputWindowHandle
        $source += '- item'
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 list source')
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf + '- '
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 list Enter continues prefix')
        [void](Save-EditorRecoveryEvidence 'E02' 'list-continuation')
        Send-Ascii 'continued item' $script:InputWindowHandle
        $source += 'continued item'
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 continued list text')
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf + '- '
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 second list prefix')
        Send-Key $script:InputWindowHandle 0x0D
        $source = $source.Substring(0, $source.Length - 2)
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 empty list Enter exits prefix')
        Send-Ascii '~~~text' $script:InputWindowHandle
        $source += '~~~text'
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 code fence opener source')
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf
        Send-Ascii 'code' $script:InputWindowHandle
        $source += 'code'
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 code body source')
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf
        Send-Ascii '~~~' $script:InputWindowHandle
        $source += '~~~'
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 closing fence source')
        Send-Key $script:InputWindowHandle 0x0D
        $source += $script:EditorRecoveryLf
        [void](Assert-EditorRecoveryDocument $source ([Text.Encoding]::UTF8.GetByteCount($source)) $true 'E02 newline after closing fence before Japanese paragraph')
        $script:EditorRecoveryExpectedSource=$source
        [void](Save-EditorRecoveryEvidence 'E02' 'markdown-complete')
        Set-EditorRecoveryCase 'E02' 'NOT_RUN' @{
            source_sha256=Get-UiTextSha256 $source; virtual_key_ascii='PASS'
            blank_line='PASS'; heading_bold='PASS'; quote_prefix_continuation_and_exit='PASS'
            list_prefix_continuation_and_exit='PASS'; fenced_code='PASS'
            workspace_auto_save_setup='PASS: owned fixture config is false; application readback NOT_EXPOSED.'
            japanese='NOT_RUN: profile-specific reading and conversion evidence is reported separately in E03/E04.'
            reason='ASCII and Markdown substeps passed; the combined row remains NOT_RUN until its separate Japanese substeps and full row criteria are reconciled.'
        }

        $script:CurrentEditorRecoveryCaseId='E03'
        $coreE03=Invoke-EditorRecoveryImeCase 'E03' 'ATOK'
        if ($null -ne $coreE03) {
            $caseE03=@($script:Report.editor_recovery_cases | Where-Object id -ceq 'E03')[0]
            Invoke-EditorRecoveryImeExtra 'E03' 'ATOK' $caseE03.details.committed_suffix $caseE03.details.expected_reading
        }
        $script:CurrentEditorRecoveryCaseId='E04'
        $coreE04=Invoke-EditorRecoveryImeCase 'E04' 'Microsoft IME'
        if ($null -ne $coreE04) {
            $caseE04=@($script:Report.editor_recovery_cases | Where-Object id -ceq 'E04')[0]
            Invoke-EditorRecoveryImeExtra 'E04' 'Microsoft IME' $caseE04.details.committed_suffix $caseE04.details.expected_reading
        }
        $state=Read-Diagnostic
        if ($state.source -cne $script:EditorRecoveryExpectedSource) { throw 'PRODUCT_ASSERTION: E04 source diverged from the exact committed IME suffixes.' }

        if ($script:Report.active_input_profile_initial) {
            $script:ImeProfile=$script:Report.active_input_profile_initial
            $restore=Request-EditorRecoveryProfile 'E04-restore' $null $script:ImeProfile
            $profileRestored = $null -ne $restore -and
                (Test-EditorRecoveryProfileIdentity $script:Report.active_input_profile_initial $restore.active_input_profile)
            $restoreState = $restore
            $modeRestored = $false
            if ($profileRestored -and $null -ne $script:Report.initial_app_local_ime_open_status.open) {
                $restoreState = $restore
                if (-not [bool]$restoreState.ime.open_status.available) {
                    $restoreState = $null
                } elseif ([bool]$restoreState.ime.open_status.open -ne [bool]$script:Report.initial_app_local_ime_open_status.open) {
                    $script:ImeInitialOpen = [bool]$script:Report.initial_app_local_ime_open_status.open
                    $script:ImeRestorePending = $true
                    $script:ImeRestoreAttempted = $false
                    Restore-ImeOpenState
                    $restoreState = Read-Diagnostic
                }
                $modeRestored = $null -ne $restoreState -and
                    [bool]$restoreState.ime.open_status.available -and
                    [bool]$restoreState.ime.open_status.open -eq [bool]$script:Report.initial_app_local_ime_open_status.open
            }
            $script:Report.ime_profile_restoration=@{
                status=if ($profileRestored -and $modeRestored) {'PASS'} else {'BLOCKED'}
                initial_profile=$script:Report.active_input_profile_initial
                final_profile=Get-EditorRecoveryProfileIdentity $(if($restoreState){$restoreState.active_input_profile}else{(Read-Diagnostic).active_input_profile})
                initial_app_local_open=$script:Report.initial_app_local_ime_open_status.open
                final_app_local_open=if($restoreState){$restoreState.ime.open_status.open}else{$null}
                profile_restored=$profileRestored; app_local_mode_restored=$modeRestored
            }
        } else { $script:Report.ime_profile_restoration=@{status='BLOCKED';reason='Initial input profile was unavailable.'} }

        $caseE02=@($script:Report.editor_recovery_cases | Where-Object {$_.id -eq 'E02'})[0]
        $caseE03=@($script:Report.editor_recovery_cases | Where-Object {$_.id -eq 'E03'})[0]
        $caseE04=@($script:Report.editor_recovery_cases | Where-Object {$_.id -eq 'E04'})[0]
        $coreSequenceStatusE03 = if ($caseE03.details -is [System.Collections.IDictionary] -and $caseE03.details.Contains('core_sequence_status')) {
            [string]$caseE03.details['core_sequence_status']
        } else { $null }
        $coreSequenceStatusE04 = if ($caseE04.details -is [System.Collections.IDictionary] -and $caseE04.details.Contains('core_sequence_status')) {
            [string]$caseE04.details['core_sequence_status']
        } else { $null }
        $caseE02.details['japanese_ime_cases']=@(
            [pscustomobject]@{id='E03';status=$caseE03.status;core_sequence_status=$coreSequenceStatusE03}
            [pscustomobject]@{id='E04';status=$caseE04.status;core_sequence_status=$coreSequenceStatusE04}
        )

        $script:CurrentEditorRecoveryCaseId='E05'
        $source=[string]$script:EditorRecoveryExpectedSource
        $state=Switch-ToSource $source
        Send-Key $script:InputWindowHandle 0x24 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoveryDocument $source 0 $true 'E05 Ctrl+Home')
        Send-Key $script:InputWindowHandle 0x27 ([uint16[]]@(0x10))
        [void](Wait-Diagnostic {param($s)$s.source -ceq $source -and [int]$s.selection.anchor -eq 0 -and [int]$s.selection.head -eq 1} 'E05 Shift+Right')
        Send-Key $script:InputWindowHandle 0x27 ([uint16[]]@(0x11,0x10))
        $state=Assert-EditorRecoverySelection $source ([long]$state.document_id) 0 6 $true ([int]$state.undo_count) ([int]$state.redo_count) 'E05 Ctrl+Shift+Right exact alpha-plus-separator span'
        $bytes=[Text.Encoding]::UTF8.GetBytes($source)
        $selected=[Text.Encoding]::UTF8.GetString($bytes,0,6)
        if ($selected -cne 'alpha ') { throw 'PRODUCT_ASSERTION: E05 Ctrl+Shift+Right did not select exact alpha plus its following space.' }
        Send-Key $script:InputWindowHandle 0x25
        [void](Wait-Diagnostic {param($s)$s.source -ceq $source -and [int]$s.selection.anchor -eq 0 -and [int]$s.selection.head -eq 0} 'E05 Left collapse')
        $e05Prefix='alpha beta'+$script:EditorRecoveryLf+$script:EditorRecoveryLf
        if (-not $source.StartsWith($e05Prefix,[StringComparison]::Ordinal)) {
            throw 'SETUP_FAILURE: exact E05 first-paragraph movement oracle is unavailable.'
        }
        $e05BlankLineStart=[Text.Encoding]::UTF8.GetByteCount('alpha beta'+$script:EditorRecoveryLf)
        $e05HeadingStart=[Text.Encoding]::UTF8.GetByteCount($e05Prefix)
        if ($e05BlankLineStart -ne 11 -or $e05HeadingStart -ne 12) {
            throw 'SETUP_FAILURE: E05 LF first-paragraph byte oracle changed.'
        }
        $e05NavigationBaseline=Read-Diagnostic
        if ($e05NavigationBaseline.source -cne $source -or
            [int]$e05NavigationBaseline.selection.anchor -ne 0 -or
            [int]$e05NavigationBaseline.selection.head -ne 0 -or
            -not [bool]$e05NavigationBaseline.dirty) {
            throw 'SETUP_FAILURE: E05 keyboard substeps require the exact dirty Core source at byte zero.'
        }
        $e05DocumentId=[long]$e05NavigationBaseline.document_id
        $e05Undo=[int]$e05NavigationBaseline.undo_count
        $e05Redo=[int]$e05NavigationBaseline.redo_count
        $e05Dirty=[bool]$e05NavigationBaseline.dirty
        $e05ImeState=Read-Diagnostic
        if (-not [bool]$e05ImeState.ime.open_status.available -or
            [bool]$e05ImeState.ime.active -or
            -not [string]::IsNullOrEmpty([string]$e05ImeState.ime.preedit) -or
            -not (Test-SameInputProfile $e05ImeState)) {
            throw 'ENVIRONMENT_BLOCKED: E05 normal ASCII replacement/sample needs an observable inactive app-local IME on the existing profile.'
        }
        if ([bool]$e05ImeState.ime.open_status.open) {
            Send-Key $script:InputWindowHandle 0x1A
            $e05ImeState=Wait-ImeOpenState $false $source
        }
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 0 $e05Dirty $e05Undo $e05Redo 'E05 unchanged profile/closed IME first-line baseline')

        # Normal Right/Left repeat reaches exact source byte 6 and returns to byte 0.
        for ($i=0; $i -lt 6; $i++) { Send-Key $script:InputWindowHandle 0x27 }
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 6 6 $e05Dirty $e05Undo $e05Redo 'E05 six normal Right keys stop after alpha and separator')
        for ($i=0; $i -lt 6; $i++) { Send-Key $script:InputWindowHandle 0x25 }
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 0 $e05Dirty $e05Undo $e05Redo 'E05 six normal Left keys return to source start')

        # Ctrl+Left/Right use the exact ASCII word boundaries 0 and 6.
        Send-Key $script:InputWindowHandle 0x27 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 6 6 $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Right after alpha')
        Send-Key $script:InputWindowHandle 0x25 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 0 $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Left before alpha')

        # Ctrl+Shift preserves selection direction: one forward and one reverse span.
        Send-Key $script:InputWindowHandle 0x27 ([uint16[]]@(0x11,0x10))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 6 $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Shift+Right forward exact span')
        Send-Key $script:InputWindowHandle 0x27
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 6 6 $e05Dirty $e05Undo $e05Redo 'E05 Right collapses forward span to its end')
        Send-Key $script:InputWindowHandle 0x25 ([uint16[]]@(0x11,0x10))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 6 0 $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Shift+Left reverse exact span')
        Send-Key $script:InputWindowHandle 0x27
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 6 6 $e05Dirty $e05Undo $e05Redo 'E05 Right collapses reverse span to its end')

        # Shift+Right and Shift+Left retain anchor/head orientation; ordinary arrows
        # collapse the selection to the directionally requested edge.
        Send-Key $script:InputWindowHandle 0x24 ([uint16[]]@(0x11))
        Send-Key $script:InputWindowHandle 0x27 ([uint16[]]@(0x10))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 1 $e05Dirty $e05Undo $e05Redo 'E05 Shift+Right forward one-byte selection')
        Send-Key $script:InputWindowHandle 0x27 ([uint16[]]@(0x10))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 2 $e05Dirty $e05Undo $e05Redo 'E05 Shift+Right extends forward selection')
        Send-Key $script:InputWindowHandle 0x25
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 0 $e05Dirty $e05Undo $e05Redo 'E05 Left collapses forward selection to start')
        Send-Key $script:InputWindowHandle 0x27
        Send-Key $script:InputWindowHandle 0x25 ([uint16[]]@(0x10))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 1 0 $e05Dirty $e05Undo $e05Redo 'E05 Shift+Left reverse one-byte selection')
        Send-Key $script:InputWindowHandle 0x27
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 1 1 $e05Dirty $e05Undo $e05Redo 'E05 Right collapses reverse selection to end')
        Send-Key $script:InputWindowHandle 0x25
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 0 $e05Dirty $e05Undo $e05Redo 'E05 Left returns after reverse selection')

        # Ctrl+Up/Down visit logical source line starts across the exact blank line.
        Send-Key $script:InputWindowHandle 0x28 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId $e05BlankLineStart $e05BlankLineStart $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Down reaches blank physical line')
        Send-Key $script:InputWindowHandle 0x28 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId $e05HeadingStart $e05HeadingStart $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Down reaches following heading line')
        Send-Key $script:InputWindowHandle 0x26 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId $e05BlankLineStart $e05BlankLineStart $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Up returns to blank physical line')
        Send-Key $script:InputWindowHandle 0x26 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 0 $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Up returns to first line')

        # Shift+Ctrl+Up over the two logical boundaries records a reverse selection.
        Send-Key $script:InputWindowHandle 0x28 ([uint16[]]@(0x11))
        Send-Key $script:InputWindowHandle 0x28 ([uint16[]]@(0x11))
        Send-Key $script:InputWindowHandle 0x26 ([uint16[]]@(0x11,0x10))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId $e05HeadingStart $e05BlankLineStart $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Shift+Up reverse selection to blank line')
        Send-Key $script:InputWindowHandle 0x26 ([uint16[]]@(0x11,0x10))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId $e05HeadingStart 0 $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Shift+Up reverse selection to source start')
        Send-Key $script:InputWindowHandle 0x25
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 0 $e05Dirty $e05Undo $e05Redo 'E05 Left collapses reverse logical-line selection')

        # Home/End are tested only on the fitting ten-ASCII-character first line.
        for ($i=0; $i -lt 3; $i++) { Send-Key $script:InputWindowHandle 0x27 }
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 3 3 $e05Dirty $e05Undo $e05Redo 'E05 Home pre-key first-line position')
        Send-Key $script:InputWindowHandle 0x24
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 0 0 $e05Dirty $e05Undo $e05Redo 'E05 Home exact first-line byte start')
        for ($i=0; $i -lt 3; $i++) { Send-Key $script:InputWindowHandle 0x27 }
        Send-Key $script:InputWindowHandle 0x23
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 10 10 $e05Dirty $e05Undo $e05Redo 'E05 End exact first-line byte end')
        [void](Save-EditorRecoveryEvidence 'E05' 'keyboard-movement-home-end')

        # Reverse selection replacement is ordinary ASCII input. One Undo restores
        # the same source and anchor=5/head=0; Redo creates exactly one group, and a
        # final Undo leaves one redo entry for the later E08 edit to discard.
        Send-Key $script:InputWindowHandle 0x24 ([uint16[]]@(0x11))
        for ($i=0; $i -lt 5; $i++) { Send-Key $script:InputWindowHandle 0x27 }
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 5 5 $e05Dirty $e05Undo $e05Redo 'E05 alpha replacement pre-key boundary')
        Send-Key $script:InputWindowHandle 0x25 ([uint16[]]@(0x11,0x10))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 5 0 $e05Dirty $e05Undo $e05Redo 'E05 Ctrl+Shift+Left reverse alpha selection')
        $reverseSelected=[Text.Encoding]::UTF8.GetString([Text.Encoding]::UTF8.GetBytes($source),0,5)
        if ($reverseSelected -cne 'alpha') { throw 'PRODUCT_ASSERTION: E05 reverse replacement did not select exact alpha.' }
        [void](Save-EditorRecoveryEvidence 'E05' 'reverse-selection-before-replacement')
        $replacement=$source.Remove(0,5).Insert(0,'Z')
        Send-Ascii 'Z' $script:InputWindowHandle
        [void](Assert-EditorRecoverySelection $replacement $e05DocumentId 1 1 $true ($e05Undo+1) 0 'E05 ordinary ASCII reverse-selection replacement')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 5 0 $e05Dirty $e05Undo 1 'E05 Undo restores reverse selection and exact source')
        [void](Save-EditorRecoveryEvidence 'E05' 'reverse-selection-undo')
        Send-Key $script:InputWindowHandle 0x59 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $replacement $e05DocumentId 1 1 $true ($e05Undo+1) 0 'E05 Redo restores exactly one replacement group')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        $e05BaseEnd=[Text.Encoding]::UTF8.GetByteCount($source)
        [void](Assert-EditorRecoverySelection $source $e05DocumentId 5 0 $e05Dirty $e05Undo 1 'E05 second Undo restores base before preferred-x sample')
        Send-Key $script:InputWindowHandle 0x23 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $source $e05DocumentId $e05BaseEnd $e05BaseEnd $e05Dirty $e05Undo 1 'E05 Ctrl+End before preferred-x sample')
        $e05BeforeSample=Read-Diagnostic
        if ($e05BeforeSample.source -cne $source -or [int]$e05BeforeSample.selection.head -ne $e05BaseEnd -or
            [int]$e05BeforeSample.redo_count -ne 1) {
            throw 'PRODUCT_ASSERTION: E05 preferred-x sample did not start at exact Core EOF with its one reverse-replacement redo.'
        }
        $e05Lf=$script:EditorRecoveryLf
        if ([Text.Encoding]::UTF8.GetByteCount($e05Lf) -ne 1) { throw 'SETUP_FAILURE: E05 preferred-x sample requires the existing one-byte LF Core EOL.' }
        $e05SampleStart=$e05BaseEnd+1
        $e05SampleUndo=[int]$e05BeforeSample.undo_count

        # Build the short three-line sample only by Enter and normal ASCII VKeys.
        Send-Key $script:InputWindowHandle 0x0D
        $e05AfterEnter1=$source+$e05Lf
        [void](Assert-EditorRecoverySelection $e05AfterEnter1 $e05DocumentId ([Text.Encoding]::UTF8.GetByteCount($e05AfterEnter1)) ([Text.Encoding]::UTF8.GetByteCount($e05AfterEnter1)) $true ($e05SampleUndo+1) 0 'E05 sample first normal Enter')
        Send-Ascii 'abcdef' $script:InputWindowHandle
        $e05AfterLine1=$e05AfterEnter1+'abcdef'
        [void](Assert-EditorRecoverySelection $e05AfterLine1 $e05DocumentId ([Text.Encoding]::UTF8.GetByteCount($e05AfterLine1)) ([Text.Encoding]::UTF8.GetByteCount($e05AfterLine1)) $true ($e05SampleUndo+2) 0 'E05 sample first normal ASCII line')
        Send-Key $script:InputWindowHandle 0x0D
        $e05AfterEnter2=$e05AfterLine1+$e05Lf
        [void](Assert-EditorRecoverySelection $e05AfterEnter2 $e05DocumentId ([Text.Encoding]::UTF8.GetByteCount($e05AfterEnter2)) ([Text.Encoding]::UTF8.GetByteCount($e05AfterEnter2)) $true ($e05SampleUndo+3) 0 'E05 sample second normal Enter')
        Send-Ascii 'x' $script:InputWindowHandle
        $e05AfterX=$e05AfterEnter2+'x'
        [void](Assert-EditorRecoverySelection $e05AfterX $e05DocumentId ([Text.Encoding]::UTF8.GetByteCount($e05AfterX)) ([Text.Encoding]::UTF8.GetByteCount($e05AfterX)) $true ($e05SampleUndo+4) 0 'E05 sample short normal ASCII line')
        Send-Key $script:InputWindowHandle 0x0D
        $e05AfterEnter3=$e05AfterX+$e05Lf
        [void](Assert-EditorRecoverySelection $e05AfterEnter3 $e05DocumentId ([Text.Encoding]::UTF8.GetByteCount($e05AfterEnter3)) ([Text.Encoding]::UTF8.GetByteCount($e05AfterEnter3)) $true ($e05SampleUndo+5) 0 'E05 sample third normal Enter')
        Send-Ascii 'abcdef' $script:InputWindowHandle
        $e05Sample=$e05AfterEnter3+'abcdef'
        [void](Assert-EditorRecoverySelection $e05Sample $e05DocumentId ([Text.Encoding]::UTF8.GetByteCount($e05Sample)) ([Text.Encoding]::UTF8.GetByteCount($e05Sample)) $true ($e05SampleUndo+6) 0 'E05 complete ordinary-typed preferred-x sample')
        [void](Save-EditorRecoveryEvidence 'E05' 'preferred-x-sample-created')
        $e05SampleState=Move-EditorRecoveryBoundaryCaret $e05Sample $e05DocumentId ($e05SampleStart+5) 0 $true ($e05SampleUndo+6) 0 'E05' 'preferred-x first-line byte 5' 3
        $e05PreferredX=[double]$e05SampleState.caret.x
        [void](Save-EditorRecoveryEvidence 'E05' 'preferred-x-first-line-start')
        Send-Key $script:InputWindowHandle 0x28
        $e05DownShort=Assert-EditorRecoverySelection $e05Sample $e05DocumentId ($e05SampleStart+8) ($e05SampleStart+8) $true ($e05SampleUndo+6) 0 'E05 Down to short-line end byte 8'
        [void](Save-EditorRecoveryEvidence 'E05' 'preferred-x-short-line')
        Send-Key $script:InputWindowHandle 0x28
        $e05DownThird=Assert-EditorRecoverySelection $e05Sample $e05DocumentId ($e05SampleStart+14) ($e05SampleStart+14) $true ($e05SampleUndo+6) 0 'E05 Down retains preferred x at third-line byte 14'
        if ([Math]::Abs([double]$e05DownThird.caret.x-$e05PreferredX) -ge 1.0) {
            throw 'PRODUCT_ASSERTION: E05 vertical Down did not restore the first-line preferred x within one DIP.'
        }
        [void](Save-EditorRecoveryEvidence 'E05' 'preferred-x-third-line')
        Send-Key $script:InputWindowHandle 0x26
        $e05UpShort=Assert-EditorRecoverySelection $e05Sample $e05DocumentId ($e05SampleStart+8) ($e05SampleStart+8) $true ($e05SampleUndo+6) 0 'E05 Up restores short-line byte 8'
        Send-Key $script:InputWindowHandle 0x26
        $e05UpFirst=Assert-EditorRecoverySelection $e05Sample $e05DocumentId ($e05SampleStart+5) ($e05SampleStart+5) $true ($e05SampleUndo+6) 0 'E05 Up returns to first-line byte 5'
        if ([Math]::Abs([double]$e05UpFirst.caret.x-$e05PreferredX) -ge 1.0) {
            throw 'PRODUCT_ASSERTION: E05 vertical Up did not restore the first-line preferred x within one DIP.'
        }
        [void](Save-EditorRecoveryEvidence 'E05' 'preferred-x-returned')

        # Shift+Up keeps a reverse directional selection across the same short line.
        # Continue normal Down from the returned byte 5; preferred_x must still be held.
        Send-Key $script:InputWindowHandle 0x28
        [void](Assert-EditorRecoverySelection $e05Sample $e05DocumentId ($e05SampleStart+8) ($e05SampleStart+8) $true ($e05SampleUndo+6) 0 'E05 Down again to short line before Shift+Up')
        Send-Key $script:InputWindowHandle 0x28
        $e05ShiftStart=Assert-EditorRecoverySelection $e05Sample $e05DocumentId ($e05SampleStart+14) ($e05SampleStart+14) $true ($e05SampleUndo+6) 0 'E05 Down again to last line before Shift+Up'
        [void](Save-EditorRecoveryEvidence 'E05' 'preferred-x-bottom-before-shift')
        Send-Key $script:InputWindowHandle 0x26 ([uint16[]]@(0x10))
        $e05ShiftUpShort=Assert-EditorRecoverySelection $e05Sample $e05DocumentId ($e05SampleStart+14) ($e05SampleStart+8) $true ($e05SampleUndo+6) 0 'E05 Shift+Up reverse selection to short-line end'
        Send-Key $script:InputWindowHandle 0x26 ([uint16[]]@(0x10))
        $e05ShiftUpFirst=Assert-EditorRecoverySelection $e05Sample $e05DocumentId ($e05SampleStart+14) ($e05SampleStart+5) $true ($e05SampleUndo+6) 0 'E05 Shift+Up preserves anchor and exact first-line head'
        [void](Save-EditorRecoveryEvidence 'E05' 'preferred-x-reverse-selection')

        # Six Undo groups are exact: three printable runs and three normal Enter edits.
        # The new typing branch deliberately discarded the one earlier replacement redo.
        $e05UndoTargets=@(
            [pscustomobject]@{source=$e05AfterEnter3;head=[Text.Encoding]::UTF8.GetByteCount($e05AfterEnter3);undo=$e05SampleUndo+5;redo=1},
            [pscustomobject]@{source=$e05AfterX;head=[Text.Encoding]::UTF8.GetByteCount($e05AfterX);undo=$e05SampleUndo+4;redo=2},
            [pscustomobject]@{source=$e05AfterEnter2;head=[Text.Encoding]::UTF8.GetByteCount($e05AfterEnter2);undo=$e05SampleUndo+3;redo=3},
            [pscustomobject]@{source=$e05AfterLine1;head=[Text.Encoding]::UTF8.GetByteCount($e05AfterLine1);undo=$e05SampleUndo+2;redo=4},
            [pscustomobject]@{source=$e05AfterEnter1;head=[Text.Encoding]::UTF8.GetByteCount($e05AfterEnter1);undo=$e05SampleUndo+1;redo=5},
            [pscustomobject]@{source=$source;head=$e05BaseEnd;undo=$e05SampleUndo;redo=6}
        )
        foreach ($target in $e05UndoTargets) {
            Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
            $dirtyAfterUndo=if ($target.undo -eq $e05SampleUndo) {$e05Dirty} else {$true}
            $state=Assert-EditorRecoverySelection ([string]$target.source) $e05DocumentId ([int]$target.head) ([int]$target.head) $dirtyAfterUndo ([int]$target.undo) ([int]$target.redo) 'E05 one exact preferred-x sample Undo group'
        }
        if ($state.source -cne $source -or [int]$state.selection.head -ne $e05BaseEnd -or [int]$state.redo_count -ne 6) {
            throw 'PRODUCT_ASSERTION: E05 six normal Undo groups did not restore the exact Core source with six redo entries.'
        }
        [void](Save-EditorRecoveryEvidence 'E05' 'preferred-x-undo-restored-core')

        # Use these diagnostic fields in E05 details/captures. They record actual
        # affinity/caret/native-caret/scroll/viewport; values at wrapped boundaries
        # must be observed at runtime, never predicted from source bytes alone.
        $e05PreferredXObservations=@(
            foreach ($observed in @($e05SampleState,$e05DownShort,$e05DownThird,$e05UpShort,$e05UpFirst,$e05ShiftStart,$e05ShiftUpShort,$e05ShiftUpFirst)) {
                [pscustomobject]@{
                    selection=$observed.selection; selection_affinity=$observed.selection_affinity
                    caret=$observed.caret; native_caret=$observed.native_caret; scroll=$observed.scroll
                    editor_viewport=$observed.parent_root.editor_rect
                }
            }
        )

        # Keep Set-EditorRecoveryCase E05 status NOT_RUN. PageUp/PageDown, actual
        # wrapped-boundary affinity review, physical typematic and the full row remain.
        Set-EditorRecoveryCase 'E05' 'NOT_RUN' @{
            verified=@(
                'normal Left/Right repeated exact byte endpoints',
                'Ctrl+Left/Right exact alpha/beta word boundary at byte 6',
                'Shift and Ctrl+Shift directional selection endpoints',
                'Ctrl+Up/Down logical-line starts at bytes 0, 11 and 12',
                'Home/End on the fitting first line at bytes 0 and 10',
                'reverse alpha replacement, one Undo, exact Redo and second Undo',
                'normal VK abcdef/LF/x/LF/abcdef preferred-x bytes 5, 8 and 14',
                'six separate sample Undo groups restore base source and expose six Redo entries'
            )
            preferred_x=@{
                sample_start_byte=$e05SampleStart
                relative_byte_heads=@(5,8,14,8,5)
                actual_caret_snapshots=$e05PreferredXObservations
                x_restoration_within_one_dip=(
                    [Math]::Abs([double]$e05DownThird.caret.x-$e05PreferredX) -lt 1.0 -and
                    [Math]::Abs([double]$e05UpFirst.caret.x-$e05PreferredX) -lt 1.0
                )
                sample_source_restored=([string]$state.source -ceq $source)
                undo_groups=6; redo_count_after_restore=[int]$state.redo_count
                previous_redo_discarded_by_first_normal_sample_edit=$true
            }
            not_run=@(
                'PageUp/PageDown byte targets: require exact pre-key DirectWrite viewport and HitTestPoint evidence',
                'ambiguous wrapped-boundary affinity: capture source/selection_affinity/caret/native caret/scroll and actual image before setting an oracle',
                'physical key hold/OS typematic'
            )
            physical_typematic='NOT_RUN'
            reason='Exact source/history VK substeps are prepared, but E05 remains NOT_RUN until page/wrap geometry review, physical typematic separation and the complete acceptance row are reconciled.'
        }

        Save-ActiveFixture $script:EditorRecoveryExpectedSource $script:EditorRecoveryBlankPath

        $script:CurrentEditorRecoveryCaseId='E06'
        $e06Source=[string]$script:EditorRecoveryPointerSource
        $e06FixtureHash=(Get-FileHash -LiteralPath $script:EditorRecoveryPointerPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($e06FixtureHash -cne [string]$script:Report.editor_recovery_pointer_fixture.copy_sha256) {
            throw 'SETUP_FAILURE: E06 owned copy of tests/fixtures/editor.md changed before use.'
        }
        $e06State=Switch-ToSource $e06Source
        $e06DocumentId=[long]$e06State.document_id
        if ([int]$e06State.tables -ne 2) {
            throw 'PRODUCT_ASSERTION: byte-exact editor.md did not expose its two expected Live tables.'
        }
        if (-not [bool]$e06State.raw) {
            Send-Key $script:InputWindowHandle 0x45 ([uint16[]]@(0x11))
            $e06State=Wait-Diagnostic {
                param($s)
                [long]$s.document_id -eq $e06DocumentId -and $s.source -ceq $e06Source -and [bool]$s.raw
            } 'E06 switch copied editor.md to Source mode'
        }
        if (-not [bool]$e06State.raw) { throw 'PRODUCT_ASSERTION: byte-exact editor.md did not enter Source mode.' }
        $e06BaseUndo=[int]$e06State.undo_count
        $e06BaseRedo=[int]$e06State.redo_count
        if ([bool]$e06State.dirty -or $e06BaseUndo -ne 0 -or $e06BaseRedo -ne 0) {
            throw 'PRODUCT_ASSERTION: copied editor.md did not start clean with empty history.'
        }
        $e06Initial=Move-EditorRecoveryPointerCaret 0 0 0 $e06DocumentId $e06BaseUndo $e06BaseRedo 'E06 source start baseline'
        [void](Save-EditorRecoveryEvidence 'E06' 'copied-fixture-source-start')

        $japanese='日本語'
        $japaneseIndex=$e06Source.IndexOf($japanese,[StringComparison]::Ordinal)
        if ($japaneseIndex -lt 0) { throw 'SETUP_FAILURE: copied editor.md lacks the fixed 日本語 selection span.' }
        $japaneseStart=[Text.Encoding]::UTF8.GetByteCount($e06Source.Substring(0,$japaneseIndex))
        $japaneseEnd=[Text.Encoding]::UTF8.GetByteCount($e06Source.Substring(0,$japaneseIndex+$japanese.Length))
        $japaneseStartState=Move-EditorRecoveryPointerCaret 6 0 $japaneseStart $e06DocumentId $e06BaseUndo $e06BaseRedo 'E06 Japanese drag start caret'
        $japaneseStartPoint=Get-EditorRecoveryCaretDipPoint $japaneseStartState 'Japanese drag start'
        for ($step=0; $step -lt 3; $step++) { Send-Key $script:InputWindowHandle 0x27 }
        $japaneseEndState=Assert-EditorRecoverySelection $e06Source $e06DocumentId $japaneseEnd $japaneseEnd $false $e06BaseUndo $e06BaseRedo 'E06 Japanese drag end caret'
        $japaneseEndPoint=Get-EditorRecoveryCaretDipPoint $japaneseEndState 'Japanese drag end'
        Invoke-UiDiagnosticDrag $japaneseStartPoint.x $japaneseStartPoint.y $japaneseEndPoint.x $japaneseEndPoint.y $japaneseStartPoint.dpi `
            -WhileHeldCondition {
                param($state)
                [bool]([int]$state.selection.anchor -eq $japaneseStart -and
                    [int]$state.selection.head -eq $japaneseEnd)
            }
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $japaneseStart $japaneseEnd $false $e06BaseUndo $e06BaseRedo 'E06 forward drag selects exact 日本語 bytes')
        [void](Save-EditorRecoveryEvidence 'E06' 'japanese-forward-drag-selection')
        $japaneseReplacement=$e06Source.Remove($japaneseIndex,$japanese.Length).Insert($japaneseIndex,'J')
        $japaneseReplacementCaret=$japaneseStart+[Text.Encoding]::UTF8.GetByteCount('J')
        Send-Ascii 'J' $script:InputWindowHandle
        [void](Assert-EditorRecoverySelection $japaneseReplacement $e06DocumentId $japaneseReplacementCaret $japaneseReplacementCaret $true ($e06BaseUndo+1) 0 'E06 normal ASCII replacement of Japanese drag selection')
        [void](Save-EditorRecoveryEvidence 'E06' 'japanese-ascii-replacement')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $japaneseStart $japaneseEnd $false $e06BaseUndo ($e06BaseRedo+1) 'E06 Undo restores Japanese source and forward selection')

        $combining='e'+[char]0x0301
        $combiningIndex=$e06Source.IndexOf($combining,[StringComparison]::Ordinal)
        if ($combiningIndex -lt 0) { throw 'SETUP_FAILURE: copied editor.md lacks the fixed decomposed e plus U+0301 span.' }
        $combiningStart=[Text.Encoding]::UTF8.GetByteCount($e06Source.Substring(0,$combiningIndex))
        $combiningEnd=[Text.Encoding]::UTF8.GetByteCount($e06Source.Substring(0,$combiningIndex+$combining.Length))
        $combiningStartState=Move-EditorRecoveryPointerCaret 7 12 $combiningStart $e06DocumentId $e06BaseUndo ($e06BaseRedo+1) 'E06 combining-mark Shift+click start caret'
        $combiningStartPoint=Get-EditorRecoveryCaretDipPoint $combiningStartState 'combining-mark selection start'
        Send-Key $script:InputWindowHandle 0x27
        $combiningEndState=Assert-EditorRecoverySelection $e06Source $e06DocumentId $combiningEnd $combiningEnd $false $e06BaseUndo ($e06BaseRedo+1) 'E06 combining-mark Shift+click end caret'
        $combiningEndPoint=Get-EditorRecoveryCaretDipPoint $combiningEndState 'combining-mark selection end'
        Invoke-UiDiagnosticClick $combiningStartPoint.x $combiningStartPoint.y $combiningStartPoint.dpi
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $combiningStart $combiningStart $false $e06BaseUndo ($e06BaseRedo+1) 'E06 click collapses at combining-mark start')
        Invoke-UiDiagnosticShiftClick $combiningEndPoint.x $combiningEndPoint.y $combiningEndPoint.dpi
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $combiningStart $combiningEnd $false $e06BaseUndo ($e06BaseRedo+1) 'E06 forward Shift+click selects decomposed é')
        [void](Save-EditorRecoveryEvidence 'E06' 'combining-forward-shift-click')
        $combiningReplacement=$e06Source.Remove($combiningIndex,$combining.Length)
        Send-Key $script:InputWindowHandle 0x2E
        [void](Assert-EditorRecoverySelection $combiningReplacement $e06DocumentId $combiningStart $combiningStart $true ($e06BaseUndo+1) 0 'E06 Delete removes one full decomposed combining grapheme')
        [void](Save-EditorRecoveryEvidence 'E06' 'combining-delete')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $combiningStart $combiningEnd $false $e06BaseUndo ($e06BaseRedo+1) 'E06 Undo restores combining bytes and forward selection')

        $womanTechnologist=[string][char]0xD83D+[char]0xDC69+[char]0x200D+[char]0xD83D+[char]0xDCBB
        $emojiIndex=$e06Source.IndexOf($womanTechnologist,[StringComparison]::Ordinal)
        if ($emojiIndex -lt 0) { throw 'SETUP_FAILURE: copied editor.md lacks the fixed ZWJ emoji span.' }
        $emojiStart=[Text.Encoding]::UTF8.GetByteCount($e06Source.Substring(0,$emojiIndex))
        $emojiEnd=[Text.Encoding]::UTF8.GetByteCount($e06Source.Substring(0,$emojiIndex+$womanTechnologist.Length))
        $emojiStartState=Move-EditorRecoveryPointerCaret 7 4 $emojiStart $e06DocumentId $e06BaseUndo ($e06BaseRedo+1) 'E06 ZWJ emoji reverse Shift+click start caret'
        $emojiStartPoint=Get-EditorRecoveryCaretDipPoint $emojiStartState 'ZWJ emoji selection start'
        Send-Key $script:InputWindowHandle 0x27
        $emojiEndState=Assert-EditorRecoverySelection $e06Source $e06DocumentId $emojiEnd $emojiEnd $false $e06BaseUndo ($e06BaseRedo+1) 'E06 ZWJ emoji reverse Shift+click end caret'
        $emojiEndPoint=Get-EditorRecoveryCaretDipPoint $emojiEndState 'ZWJ emoji selection end'
        Invoke-UiDiagnosticClick $emojiEndPoint.x $emojiEndPoint.y $emojiEndPoint.dpi
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $emojiEnd $emojiEnd $false $e06BaseUndo ($e06BaseRedo+1) 'E06 click collapses at ZWJ emoji end')
        Invoke-UiDiagnosticShiftClick $emojiStartPoint.x $emojiStartPoint.y $emojiStartPoint.dpi
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $emojiEnd $emojiStart $false $e06BaseUndo ($e06BaseRedo+1) 'E06 reverse Shift+click selects full 👩‍💻 bytes')
        [void](Save-EditorRecoveryEvidence 'E06' 'zwj-emoji-reverse-shift-click')
        $emojiReplacement=$e06Source.Remove($emojiIndex,$womanTechnologist.Length)
        Send-Key $script:InputWindowHandle 0x08
        [void](Assert-EditorRecoverySelection $emojiReplacement $e06DocumentId $emojiStart $emojiStart $true ($e06BaseUndo+1) 0 'E06 Backspace removes one full reverse-selected ZWJ emoji')
        [void](Save-EditorRecoveryEvidence 'E06' 'zwj-emoji-backspace')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $emojiEnd $emojiStart $false $e06BaseUndo ($e06BaseRedo+1) 'E06 Undo restores ZWJ bytes and reverse selection')

        $e06Length=[Text.Encoding]::UTF8.GetByteCount($e06Source)
        Send-Key $script:InputWindowHandle 0x41 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId 0 $e06Length $false $e06BaseUndo ($e06BaseRedo+1) 'E06 Ctrl+A selects exact full UTF-8 source')
        [void](Save-EditorRecoveryEvidence 'E06' 'ctrl-a-full-source-selection')
        Send-Key $script:InputWindowHandle 0x2E
        [void](Assert-EditorRecoverySelection '' $e06DocumentId 0 0 $true ($e06BaseUndo+1) 0 'E06 Delete clears Ctrl+A selection')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId 0 $e06Length $false $e06BaseUndo ($e06BaseRedo+1) 'E06 Undo restores Ctrl+A source and selection direction')
        [void](Save-EditorRecoveryEvidence 'E06' 'ctrl-a-delete-undo')
        # Insert after the existing E06 clean-source guard and before Set-EditorRecoveryCase E06 / E07.
        # This uses the current TEMP editor.md copy; all document changes are normal VK input and Undo only.
        $e06WrapBaseline=Read-Diagnostic
        $e06WrapBaselineBytes=[Text.Encoding]::UTF8.GetByteCount($e06Source)
        if ([long]$e06WrapBaseline.document_id -ne $e06DocumentId -or
            [string]$e06WrapBaseline.source -cne $e06Source -or
            $e06WrapBaselineBytes -ne 673 -or
            [bool]$e06WrapBaseline.dirty -or
            [int]$e06WrapBaseline.undo_count -ne $e06BaseUndo -or
            [int]$e06WrapBaseline.redo_count -ne ($e06BaseRedo+1) -or
            -not [bool]$e06WrapBaseline.raw) {
            throw 'SETUP_FAILURE: E06 wrapped selection requires the same clean 673-byte TEMP editor.md Source checkpoint with its existing one-entry redo branch.'
        }
        $e06WrapOwnedSettingsPaths=@(
            [string]$script:EditorRecoveryWorkspaceSettingsPath
            (Join-Path $script:UiLocalAppData 'MDLite/settings.toml')
            (Join-Path (Join-Path $script:UiLocalAppData 'Roaming') 'MDLite/settings.toml')
        )
        $e06WrapOwnedSettingsBefore=@(
            foreach ($path in $e06WrapOwnedSettingsPaths) {
                $exists=Test-Path -LiteralPath $path -PathType Leaf
                [pscustomobject]@{path=$path;exists=$exists;sha256=(Get-UiSafeFileHash $path)}
            }
        )

        $e06WrapLines=[regex]::Split($e06Source,'\n')
        if ($e06WrapLines.Length -lt 18) { throw 'SETUP_FAILURE: E06 TEMP editor.md has no physical line 18.' }
        $e06WrapLine18Char=0
        for ($line=0; $line -lt 17; $line++) {
            $newline=$e06Source.IndexOf([char]10,$e06WrapLine18Char)
            if ($newline -lt 0) { throw 'SETUP_FAILURE: E06 physical line 18 start is not present in the exact source.' }
            $e06WrapLine18Char=$newline+1
        }
        $e06WrapParagraph=[string]$e06WrapLines[17]
        if ($e06WrapParagraph.EndsWith([char]13)) { $e06WrapParagraph=$e06WrapParagraph.Substring(0,$e06WrapParagraph.Length-1) }
        $e06WrapParagraphTrimmed=$e06WrapParagraph.TrimStart()
        $e06WrapTicks=([string][char]96)*3
        $e06WrapFenceOpen=$false
        $e06WrapPriorLines=[regex]::Split($e06Source.Substring(0,$e06WrapLine18Char),'\n')
        foreach ($line in $e06WrapPriorLines) {
            $trimmed=[string]$line
            if ($trimmed.EndsWith([char]13)) { $trimmed=$trimmed.Substring(0,$trimmed.Length-1) }
            $trimmed=$trimmed.TrimStart()
            if ($trimmed.StartsWith($e06WrapTicks,[StringComparison]::Ordinal) -or
                $trimmed.StartsWith('~~~',[StringComparison]::Ordinal)) {
                $e06WrapFenceOpen=-not $e06WrapFenceOpen
            }
        }
        if ([string]::IsNullOrWhiteSpace($e06WrapParagraph) -or $e06WrapFenceOpen -or
            $e06WrapParagraphTrimmed.StartsWith('#',[StringComparison]::Ordinal) -or
            $e06WrapParagraphTrimmed.StartsWith('>',[StringComparison]::Ordinal) -or
            $e06WrapParagraphTrimmed.StartsWith('|',[StringComparison]::Ordinal) -or
            $e06WrapParagraphTrimmed.StartsWith('- ',[StringComparison]::Ordinal) -or
            $e06WrapParagraphTrimmed.StartsWith('* ',[StringComparison]::Ordinal) -or
            $e06WrapParagraphTrimmed.StartsWith('+ ',[StringComparison]::Ordinal) -or
            $e06WrapParagraphTrimmed.StartsWith($e06WrapTicks,[StringComparison]::Ordinal) -or
            $e06WrapParagraphTrimmed.StartsWith('~~~',[StringComparison]::Ordinal) -or
            $e06WrapParagraphTrimmed -match '^\d+[.)]\s') {
            throw 'SETUP_FAILURE: E06 physical line 18 is not an ordinary paragraph outside a fenced or structural Markdown block.'
        }
        $e06WrapStartByte=[Text.Encoding]::UTF8.GetByteCount($e06Source.Substring(0,$e06WrapLine18Char))
        $e06WrapAscii='wrap boundary marker alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo sierra tango '
        if ($e06WrapAscii -notmatch '^[a-z ]+$') { throw 'SETUP_FAILURE: E06 wrap payload must remain ordinary printable ASCII prose.' }
        $e06WrapBytes=[Text.Encoding]::UTF8.GetByteCount($e06WrapAscii)
        if ($e06WrapBytes -ne $e06WrapAscii.Length -or $e06WrapBytes -ne 145) { throw 'SETUP_FAILURE: E06 wrap payload byte oracle changed.' }
        $e06WrapEndByte=$e06WrapStartByte+$e06WrapBytes
        $e06WrapBaseUndo=[int]$e06WrapBaseline.undo_count

        # Normal source navigation establishes the fixed insertion point; no file/body is synthesized or written.
        Send-Key $script:InputWindowHandle 0x24 ([uint16[]]@(0x11))
        for ($line=0; $line -lt 17; $line++) { Send-Key $script:InputWindowHandle 0x28 ([uint16[]]@(0x11)) }
        $e06WrapInsertPoint=Assert-EditorRecoverySelection $e06Source $e06DocumentId $e06WrapStartByte $e06WrapStartByte $false $e06WrapBaseUndo ($e06BaseRedo+1) 'E06 normal navigation to physical line 18 insertion start'
        if (-not [bool]$e06WrapInsertPoint.raw) { throw 'PRODUCT_ASSERTION: E06 insertion point left exact Source mode.' }
        Send-Ascii $e06WrapAscii $script:InputWindowHandle
        $e06WrapSource=$e06Source.Insert($e06WrapLine18Char,$e06WrapAscii)
        [void](Assert-EditorRecoverySelection $e06WrapSource $e06DocumentId $e06WrapEndByte $e06WrapEndByte $true ($e06WrapBaseUndo+1) 0 'E06 one ordinary ASCII paragraph insertion clears incoming redo')
        [void](Save-EditorRecoveryEvidence 'E06' 'wrapped-paragraph-after-normal-ascii-insertion')

        # Capture both points from the same modified source by returning to line 18 normally.
        Send-Key $script:InputWindowHandle 0x24 ([uint16[]]@(0x11))
        for ($line=0; $line -lt 17; $line++) { Send-Key $script:InputWindowHandle 0x28 ([uint16[]]@(0x11)) }
        $e06WrapStartState=Assert-EditorRecoverySelection $e06WrapSource $e06DocumentId $e06WrapStartByte $e06WrapStartByte $true ($e06WrapBaseUndo+1) 0 'E06 fresh wrapped-source insertion-start caret'
        $e06WrapStartPoint=Get-EditorRecoveryCaretDipPoint $e06WrapStartState 'E06 wrapped insertion start'
        for ($step=0; $step -lt $e06WrapBytes; $step++) { Send-Key $script:InputWindowHandle 0x27 }
        $e06WrapEndState=Assert-EditorRecoverySelection $e06WrapSource $e06DocumentId $e06WrapEndByte $e06WrapEndByte $true ($e06WrapBaseUndo+1) 0 'E06 fresh wrapped-source insertion-end caret'
        $e06WrapEndPoint=Get-EditorRecoveryCaretDipPoint $e06WrapEndState 'E06 wrapped insertion end'
        $e06WrapOwnedSettingsAtPoints=@(
            foreach ($path in $e06WrapOwnedSettingsPaths) {
                $exists=Test-Path -LiteralPath $path -PathType Leaf
                [pscustomobject]@{path=$path;exists=$exists;sha256=(Get-UiSafeFileHash $path)}
            }
        )
        $e06WrapOwnedSettingsBeforeJson=ConvertTo-Json -InputObject $e06WrapOwnedSettingsBefore -Depth 5 -Compress
        $e06WrapOwnedSettingsAtPointsJson=ConvertTo-Json -InputObject $e06WrapOwnedSettingsAtPoints -Depth 5 -Compress
        if ([long]$e06WrapStartState.document_id -ne [long]$e06WrapEndState.document_id -or
            [long]$e06WrapStartState.revision -ne [long]$e06WrapEndState.revision -or
            [string]$e06WrapStartState.source -cne [string]$e06WrapEndState.source -or
            -not [bool]$e06WrapStartState.raw -or -not [bool]$e06WrapEndState.raw -or
            $e06WrapOwnedSettingsBeforeJson -cne $e06WrapOwnedSettingsAtPointsJson -or
            ([string](ConvertTo-Json -InputObject $e06WrapStartState.scroll -Depth 5 -Compress)) -cne
                ([string](ConvertTo-Json -InputObject $e06WrapEndState.scroll -Depth 5 -Compress)) -or
            ([string](ConvertTo-Json -InputObject $e06WrapStartState.parent_root.active_editor_rect -Depth 5 -Compress)) -cne
                ([string](ConvertTo-Json -InputObject $e06WrapEndState.parent_root.active_editor_rect -Depth 5 -Compress)) -or
            [double]$e06WrapStartPoint.dpi -ne [double]$e06WrapEndPoint.dpi -or
            ([string](ConvertTo-Json -InputObject $e06WrapStartPoint.native_size_px -Compress)) -cne
                ([string](ConvertTo-Json -InputObject $e06WrapEndPoint.native_size_px -Compress))) {
            throw 'ENVIRONMENT_BLOCKED: E06 wrap endpoints did not share exact source/document/revision/viewport/DPI/scroll/native-caret metrics and pinned settings state.'
        }
        $e06WrapLineHeightDip=[double]$e06WrapStartPoint.native_size_px[1]/[double]$e06WrapStartPoint.dpi
        $e06WrapDy=[double]$e06WrapEndPoint.y-[double]$e06WrapStartPoint.y
        if ($e06WrapDy -lt ($e06WrapLineHeightDip*0.75) -or $e06WrapDy -gt ($e06WrapLineHeightDip*2.25)) {
            throw 'ENVIRONMENT_BLOCKED: the E06 sample/current viewport did not provide two-to-three wrapped rows; do not drag or claim wrap coverage for this setup.'
        }
        [void](Save-EditorRecoveryEvidence 'E06' 'wrapped-paragraph-same-view-points-before-drag')
        Invoke-UiDiagnosticDrag $e06WrapStartPoint.x $e06WrapStartPoint.y $e06WrapEndPoint.x $e06WrapEndPoint.y $e06WrapStartPoint.dpi `
            -WhileHeldCondition {
                param($state)
                [bool]([int]$state.selection.anchor -eq $e06WrapStartByte -and
                    [int]$state.selection.head -eq $e06WrapEndByte)
            }
        $e06WrapDragState=Assert-EditorRecoverySelection $e06WrapSource $e06DocumentId $e06WrapStartByte $e06WrapEndByte $true ($e06WrapBaseUndo+1) 0 'E06 owned cross-wrap drag selects exact ordinary paragraph bytes'
        $e06WrapSourceBytes=[Text.Encoding]::UTF8.GetBytes($e06WrapSource)
        $e06WrapSelected=[Text.Encoding]::UTF8.GetString($e06WrapSourceBytes,$e06WrapStartByte,$e06WrapEndByte-$e06WrapStartByte)
        if ($e06WrapSelected -cne $e06WrapAscii) { throw 'PRODUCT_ASSERTION: E06 cross-wrap drag did not select exactly the inserted ASCII span.' }
        [void](Save-EditorRecoveryEvidence 'E06' 'wrapped-paragraph-owned-drag-selection')

        # Replace the exact forward selection, then Undo twice. The first normal edit discards the old redo branch.
        $e06WrapReplacement=$e06WrapSource.Remove($e06WrapLine18Char,$e06WrapAscii.Length).Insert($e06WrapLine18Char,'Z')
        Send-Ascii 'Z' $script:InputWindowHandle
        [void](Assert-EditorRecoverySelection $e06WrapReplacement $e06DocumentId ($e06WrapStartByte+1) ($e06WrapStartByte+1) $true ($e06WrapBaseUndo+2) 0 'E06 normal ASCII replacement of cross-wrap selection')
        [void](Save-EditorRecoveryEvidence 'E06' 'wrapped-paragraph-ascii-replacement')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $e06WrapSource $e06DocumentId $e06WrapStartByte $e06WrapEndByte $true ($e06WrapBaseUndo+1) 1 'E06 first Undo restores exact wrapped source and forward selection')
        [void](Save-EditorRecoveryEvidence 'E06' 'wrapped-paragraph-first-undo')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        $e06WrapFinal=Assert-EditorRecoverySelection $e06Source $e06DocumentId $e06WrapStartByte $e06WrapStartByte $false $e06WrapBaseUndo 2 'E06 second Undo restores exact 673-byte clean fixture and exposes two Redo entries'
        $e06WrapDiskHash=(Get-FileHash -LiteralPath $script:EditorRecoveryPointerPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $e06WrapOwnedSettingsAfter=@(
            foreach ($path in $e06WrapOwnedSettingsPaths) {
                $exists=Test-Path -LiteralPath $path -PathType Leaf
                [pscustomobject]@{path=$path;exists=$exists;sha256=(Get-UiSafeFileHash $path)}
            }
        )
        $e06WrapOwnedSettingsAfterJson=ConvertTo-Json -InputObject $e06WrapOwnedSettingsAfter -Depth 5 -Compress
        if ([string]$e06WrapFinal.source -cne $e06Source -or [bool]$e06WrapFinal.dirty -or
            [int]$e06WrapFinal.undo_count -ne $e06WrapBaseUndo -or [int]$e06WrapFinal.redo_count -ne 2 -or
            $e06WrapDiskHash -cne $e06FixtureHash -or -not [bool]$e06WrapFinal.raw) {
            throw 'PRODUCT_ASSERTION: E06 wrapped extension did not restore the exact clean TEMP fixture without saving it.'
        }
        if ($e06WrapOwnedSettingsBeforeJson -cne $e06WrapOwnedSettingsAfterJson) {
            throw 'ENVIRONMENT_BLOCKED: an owned workspace/Common/AppData settings file changed during E06 wrapped selection.'
        }
        $e06Final=$e06WrapFinal
        $e06WrapSelectionDetails=@{
            runtime_source_history='PASS only after these exact normal-input, owned-drag and two-Undo assertions execute'
            source_sha256=Get-UiTextSha256 $e06Source
            baseline_source_bytes=$e06WrapBaselineBytes
            insertion_start_byte=$e06WrapStartByte
            insertion_end_byte=$e06WrapEndByte
            inserted_ascii_bytes=$e06WrapBytes
            start_point=$e06WrapStartPoint
            end_point=$e06WrapEndPoint
            same_source_document_revision_viewport_dpi_scroll=$true
            owned_workspace_common_appdata_settings_unchanged=$true
            rendered_font_identity='UNOBSERVED'
            owned_settings_before=$e06WrapOwnedSettingsBefore
            owned_settings_after=$e06WrapOwnedSettingsAfter
            observed_wrapped_row_delta_dip=$e06WrapDy
            selected_source_sha256=Get-UiTextSha256 $e06WrapSelected
            replacement_source_sha256=Get-UiTextSha256 $e06WrapReplacement
            undo_final=@{source_sha256=Get-UiTextSha256 ([string]$e06WrapFinal.source);dirty=[bool]$e06WrapFinal.dirty;undo_count=[int]$e06WrapFinal.undo_count;redo_count=[int]$e06WrapFinal.redo_count}
            incoming_redo_discarded_by_first_edit=$true
            temp_fixture_disk_unchanged=($e06WrapDiskHash -ceq $e06FixtureHash)
            live_or_folded_view_visual_review='NOT_RUN'
        }
        $e06Final=Read-Diagnostic
        if ([string]$e06Final.source -cne $e06Source -or [bool]$e06Final.dirty -or -not [bool]$e06Final.raw) {
            throw 'PRODUCT_ASSERTION: E06 edits did not return the owned copied fixture to its exact clean source.'
        }
        Set-EditorRecoveryCase 'E06' 'NOT_RUN' @{
            fixture_sha256=$e06FixtureHash; original_source_sha256=Get-UiTextSha256 $e06Source
            verified=@('forward drag of Japanese 日本語 then normal ASCII replacement/Undo',
                'forward Shift+click of decomposed e plus U+0301 then Delete/Undo',
                'reverse Shift+click of 👩‍💻 then Backspace/Undo',
                'Ctrl+A then Delete/Undo of exact full source',
                'machine-only wrapped paragraph line18 insertion, fresh endpoints, owned drag/source/history Undo/Undo, and settings-hash stability')
            wrapped_selection_machine=$e06WrapSelectionDetails
            selected_utf8_ranges=@{
                japanese=@{start=$japaneseStart;end=$japaneseEnd}
                combining=@{start=$combiningStart;end=$combiningEnd}
                zwj_emoji=@{start=$emojiStart;end=$emojiEnd}
            }
            restored_source_sha256=Get-UiTextSha256 ([string]$e06Final.source)
            restored_clean=$true; undo_count=[int]$e06Final.undo_count; redo_count=[int]$e06Final.redo_count
            not_run=@('Root review of actual wrapped-selection captures and Source+Live/folded-view visual presentation')
            reason='Wrapped selection source/history machine substeps are recorded; E06 remains NOT_RUN pending Root image review and complete visual acceptance criteria.'
        }

        $script:CurrentEditorRecoveryCaseId='E07'
        [void](Switch-ToSource $e06Source)
        $e07Initial=Move-EditorRecoveryPointerCaret 0 0 0 $e06DocumentId $e06BaseUndo ([int]$e06Final.redo_count) 'E07 exact source fixture baseline'
        if (-not [bool]$e07Initial.raw) {
            Send-Key $script:InputWindowHandle 0x45 ([uint16[]]@(0x11))
            $e07Initial=Wait-Diagnostic {
                param($s)
                [long]$s.document_id -eq $e06DocumentId -and $s.source -ceq $e06Source -and [bool]$s.raw
            } 'E07 Source mode for physical-line indentation cases'
        }
        $e07LineCases=@(
            [pscustomobject]@{name='ordinary paragraph';number=18;text='後続の段落です。'}
            [pscustomobject]@{name='quote';number=22;text='> 引用の本文です。'}
            [pscustomobject]@{name='list';number=24;text='- 一つ目の項目'}
            [pscustomobject]@{name='code';number=28;text='let source = "Markdown";'}
        )
        $e07Operations=@()
        $e07Undo=[int]$e07Initial.undo_count
        $e07Redo=[int]$e07Initial.redo_count
        foreach ($lineCase in $e07LineCases) {
            $operation=Invoke-EditorRecoveryIndentationLine $e06Source $e06DocumentId ([int]$lineCase.number) ([string]$lineCase.text) $e07Undo $e07Redo
            $operation | Add-Member -NotePropertyName kind -NotePropertyValue ([string]$lineCase.name)
            $e07Operations += $operation
            $e07Undo=[int]$operation.undo_count_after
            $e07Redo=[int]$operation.redo_count_after
        }

        $taskLineNumber=24
        $taskLines=[regex]::Split($e06Source,'\n')
        $taskLineText=[string]$taskLines[$taskLineNumber-1]
        if ($taskLineText -cne '- 一つ目の項目') { throw 'SETUP_FAILURE: fixed task-list source anchor changed.' }
        $taskLineStartChar=0
        for ($lineIndex=0; $lineIndex -lt ($taskLineNumber-1); $lineIndex++) {
            $taskLineStartChar += $taskLines[$lineIndex].Length + 1
        }
        $taskLineStartByte=[Text.Encoding]::UTF8.GetByteCount($e06Source.Substring(0,$taskLineStartChar))
        Send-Key $script:InputWindowHandle 0x24 ([uint16[]]@(0x11))
        for ($lineIndex=0; $lineIndex -lt ($taskLineNumber-1); $lineIndex++) {
            Send-Key $script:InputWindowHandle 0x28 ([uint16[]]@(0x11))
        }
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $taskLineStartByte $taskLineStartByte $false $e07Undo $e07Redo 'E07 task-list marker source line start')
        Send-Key $script:InputWindowHandle 0x27
        Send-Key $script:InputWindowHandle 0x27
        $taskMarkerStartChar=$taskLineStartChar+2
        $taskMarkerStartByte=$taskLineStartByte+2
        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $taskMarkerStartByte $taskMarkerStartByte $false $e07Undo $e07Redo 'E07 task-list marker insertion point after "- "')
        Send-Ascii '[ ] ' $script:InputWindowHandle
        $taskSource=$e06Source.Insert($taskMarkerStartChar,'[ ] ')
        $taskText=$taskLineText.Insert(2,'[ ] ')
        $taskLineEndByte=$taskLineStartByte+[Text.Encoding]::UTF8.GetByteCount($taskText)
        $taskMarkerCaretByte=$taskMarkerStartByte+[Text.Encoding]::UTF8.GetByteCount('[ ] ')
        [void](Assert-EditorRecoverySelection $taskSource $e06DocumentId $taskMarkerCaretByte $taskMarkerCaretByte $true ($e07Undo+1) 0 'E07 normal ASCII task-list marker input leaves caret after marker')
        [void](Save-EditorRecoveryEvidence 'E07' 'task-list-marker-caret')
        Send-Key $script:InputWindowHandle 0x24
        Send-Key $script:InputWindowHandle 0x23 ([uint16[]]@(0x10))
        [void](Assert-EditorRecoverySelection $taskSource $e06DocumentId $taskLineStartByte $taskLineEndByte $true ($e07Undo+1) 0 'E07 select task-list line')
        $taskIndentedSource=$taskSource.Insert($taskLineStartChar,'    ')
        Send-Key $script:InputWindowHandle 0x09
        $taskIndented=Assert-EditorRecoverySelection $taskIndentedSource $e06DocumentId ($taskLineStartByte+4) ($taskLineEndByte+4) $true ($e07Undo+2) 0 'E07 Tab indents task-list item'
        Send-Key $script:InputWindowHandle 0x09 ([uint16[]]@(0x10))
        $taskUnindented=Assert-EditorRecoverySelection $taskSource $e06DocumentId $taskLineStartByte $taskLineEndByte $true ($e07Undo+3) 0 'E07 Shift+Tab restores task-list item'
        [void](Save-EditorRecoveryEvidence 'E07' 'task-list-shift-tab-restored')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $taskIndentedSource $e06DocumentId ($taskLineStartByte+4) ($taskLineEndByte+4) $true ($e07Undo+2) 1 'E07 Undo task Shift+Tab')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $taskSource $e06DocumentId $taskLineStartByte $taskLineEndByte $true ($e07Undo+1) 2 'E07 Undo task Tab')
        Send-Key $script:InputWindowHandle 0x27
        [void](Assert-EditorRecoverySelection $taskSource $e06DocumentId $taskLineEndByte $taskLineEndByte $true ($e07Undo+1) 2 'E07 collapse task-line selection to end before Enter')
        $taskLineEndChar=$taskLineStartChar+$taskText.Length
        $taskEnterContinuationSuffix=$script:EditorRecoveryLf+'- [ ] '
        $taskEnterContinuationSource=$taskSource.Insert($taskLineEndChar,$taskEnterContinuationSuffix)
        $taskEnterContinuationCaretByte=$taskLineEndByte+[Text.Encoding]::UTF8.GetByteCount($taskEnterContinuationSuffix)
        Send-Key $script:InputWindowHandle 0x0D
        $taskContinuation=Assert-EditorRecoverySelection $taskEnterContinuationSource $e06DocumentId `
            $taskEnterContinuationCaretByte $taskEnterContinuationCaretByte $true ($e07Undo+2) 0 'E07 Enter continues task into an empty checklist item'
        [void](Save-EditorRecoveryEvidence 'E07' 'task-empty-continuation-enter')
        $taskEmptyLineStartByte=$taskLineEndByte+[Text.Encoding]::UTF8.GetByteCount($script:EditorRecoveryLf)
        $taskEnterExitSource=$taskSource.Insert($taskLineEndChar,$script:EditorRecoveryLf)
        Send-Key $script:InputWindowHandle 0x0D
        $taskEmptyExit=Assert-EditorRecoverySelection $taskEnterExitSource $e06DocumentId `
            $taskEmptyLineStartByte $taskEmptyLineStartByte $true ($e07Undo+3) 0 'E07 Enter on empty task item exits task list'
        [void](Save-EditorRecoveryEvidence 'E07' 'task-empty-enter-exit')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $taskEnterContinuationSource $e06DocumentId `
            $taskEnterContinuationCaretByte $taskEnterContinuationCaretByte $true ($e07Undo+2) 1 'E07 Undo empty-task exit restores checklist item')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoverySelection $taskSource $e06DocumentId `
            $taskLineEndByte $taskLineEndByte $true ($e07Undo+1) 2 'E07 Undo task continuation restores original task line')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        $e07Final=Assert-EditorRecoverySelection $e06Source $e06DocumentId $taskMarkerStartByte $taskMarkerStartByte $false $e07Undo 3 'E07 Undo task marker restores exact repository fixture'
        [void](Save-EditorRecoveryEvidence 'E07' 'task-enter-undo-restored-fixture')
        Set-EditorRecoveryCase 'E07' 'NOT_RUN' @{
            source_sha256=Get-UiTextSha256 $e06Source
            verified=@('Tab/Shift+Tab on ordinary paragraph, quote, bullet list, and code line','normal ASCII [ ] insertion and task-list indentation/Undo','Enter continues a nonempty task item; Enter on the empty checklist item exits; Undo restores the exact fixture')
            line_operations=$e07Operations
            task_list=@{
                source_after_marker=Get-UiTextSha256 $taskSource
                source_after_tab=Get-UiTextSha256 ([string]$taskIndented.source)
                source_after_shift_tab=Get-UiTextSha256 ([string]$taskUnindented.source)
                source_after_enter_continuation=Get-UiTextSha256 ([string]$taskContinuation.source)
                source_after_empty_task_exit=Get-UiTextSha256 ([string]$taskEmptyExit.source)
                continuation_caret_byte=$taskEnterContinuationCaretByte
                empty_task_exit_caret_byte=$taskEmptyLineStartByte
                undo_restored_original=([string]$e07Final.source -ceq $e06Source)
                final_selection=@([int]$e07Final.selection.anchor,[int]$e07Final.selection.head)
            }
            final_dirty=[bool]$e07Final.dirty; final_undo_count=[int]$e07Final.undo_count; final_redo_count=[int]$e07Final.redo_count
            not_run=@('Enter and indentation combinations across all remaining E07 paragraph/list/quote/code variants')
            reason='Task continuation, empty-task exit, and indentation substeps are automated on the owned exact fixture; the complete combined E07 row remains NOT_RUN.'
        }

        [void](Assert-EditorRecoverySelection $e06Source $e06DocumentId $taskMarkerStartByte $taskMarkerStartByte $false $e07Undo 3 'E07 clean byte-exact fixture before close')
        Send-Key $script:InputWindowHandle 0x57 ([uint16[]]@(0x11))
        [void](Wait-Diagnostic {param($s)[int]$s.tabs -eq 2} 'close clean temporary pointer fixture before E13')
        $pointerDiskHash=(Get-FileHash -LiteralPath $script:EditorRecoveryPointerPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $pointerAssetDiskHash=(Get-FileHash -LiteralPath $script:EditorRecoveryPointerAssetPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $workspaceDiskHash=(Get-FileHash -LiteralPath $script:EditorRecoveryWorkspacePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($pointerDiskHash -cne [string]$script:Report.editor_recovery_pointer_fixture.copy_sha256 -or
            $pointerAssetDiskHash -cne [string]$script:Report.editor_recovery_pointer_fixture.asset_sha256 -or
            $workspaceDiskHash -cne [string]$script:EditorRecoveryWorkspaceInitialHash) {
            throw 'PRODUCT_ASSERTION: E06/E07 changed the copied editor fixture, its SVG asset, or the original BOM/CRLF Workspace file on disk.'
        }
        $e07Resume=Switch-ToSource $script:EditorRecoveryExpectedSource
        if ([int]$e07Resume.tabs -ne 2 -or $e07Resume.source -cne $script:EditorRecoveryExpectedSource -or [bool]$e07Resume.dirty) {
            throw 'PRODUCT_ASSERTION: original two-tab Workspace and exact clean Core source were not restored before E13.'
        }
        [void](Save-EditorRecoveryEvidence 'E07' 'original-two-tabs-restored-before-e13')

        Invoke-EditorRecoveryE08

        $script:CurrentEditorRecoveryCaseId='E09'
        $e08Returned=Switch-ToSource $script:EditorRecoveryExpectedSource
        [void](Move-EditorRecoveryBoundaryCaret $script:EditorRecoveryExpectedSource ([long]$e08Returned.document_id) ([Text.Encoding]::UTF8.GetByteCount($script:EditorRecoveryExpectedSource)) 0 $false ([int]$e08Returned.undo_count) ([int]$e08Returned.redo_count) 'E09' 'post-E08 Core EOF')
        $e09ImeState=Read-Diagnostic
        if (-not [bool]$e09ImeState.ime.open_status.available) { throw 'ENVIRONMENT_BLOCKED: E09 cannot observe the app-local IME open state after E08.' }
        if (-not (Test-EditorRecoveryProfileIdentity $script:Report.active_input_profile_initial $e09ImeState.active_input_profile) -or -not (Test-SameInputProfile $e09ImeState)) { throw 'ENVIRONMENT_BLOCKED: E09 active input profile changed after E08.' }
        if ($e09ImeState.source -cne $script:EditorRecoveryExpectedSource) { throw 'PRODUCT_ASSERTION: E09 source changed after E08.' }
        if ([bool]$e09ImeState.ime.active -or -not [string]::IsNullOrEmpty([string]$e09ImeState.ime.preedit)) { throw 'ENVIRONMENT_BLOCKED: E09 cannot send ASCII while IME composition is active.' }
        if ([bool]$e09ImeState.ime.open_status.open) {
            Send-Key $script:InputWindowHandle 0x1A
            $e09ImeState=Wait-ImeOpenState $false ([string]$script:EditorRecoveryExpectedSource)
        }
        if (-not [bool]$e09ImeState.ime.open_status.available -or [bool]$e09ImeState.ime.open_status.open -or [bool]$e09ImeState.ime.active -or -not [string]::IsNullOrEmpty([string]$e09ImeState.ime.preedit) -or -not (Test-EditorRecoveryProfileIdentity $script:Report.active_input_profile_initial $e09ImeState.active_input_profile)) { throw 'ENVIRONMENT_BLOCKED: E09 requires a freshly observed closed app-local IME on the initial profile.' }
        $base=[string]$script:EditorRecoveryExpectedSource
        $state=Switch-ToSource $base
        Send-Ascii ' tail' $script:InputWindowHandle
        $withText=$base+' tail'; $textEnd=[Text.Encoding]::UTF8.GetByteCount($withText)
        [void](Assert-EditorRecoveryDocument $withText $textEnd $true 'E09 ASCII after IME and movement')
        Send-Key $script:InputWindowHandle 0x0D
        $withEnter=$withText+$script:EditorRecoveryLf; $enterEnd=[Text.Encoding]::UTF8.GetByteCount($withEnter)
        [void](Assert-EditorRecoveryDocument $withEnter $enterEnd $true 'E09 Enter after IME')
        Send-Key $script:InputWindowHandle 0x23 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoveryDocument $withEnter $enterEnd $true 'E09 Ctrl+End')
        Send-Key $script:InputWindowHandle 0x25 ([uint16[]]@(0x10))
        [void](Wait-Diagnostic {param($s)$s.source -ceq $withEnter -and [int]$s.selection.anchor -eq $enterEnd -and [int]$s.selection.head -eq ($enterEnd-1)} 'E09 newline selection')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoveryDocument $withText $textEnd $true 'E09 Undo Enter')
        Send-Key $script:InputWindowHandle 0x59 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoveryDocument $withEnter $enterEnd $true 'E09 Redo Enter')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        [void](Assert-EditorRecoveryDocument $withText $textEnd $true 'E09 second Undo')
        Save-ActiveFixture $withText $script:EditorRecoveryBlankPath
        $script:EditorRecoveryExpectedSource=$withText
        [void](Save-EditorRecoveryEvidence 'E09' 'ime-undo-redo-save')
        Set-EditorRecoveryCase 'E09' 'NOT_RUN' @{
            source_before=Get-UiTextSha256 $base; source_undo=Get-UiTextSha256 $withText
            source_redo=Get-UiTextSha256 $withEnter; committed_suffix_preserved=$withText.StartsWith($base,[StringComparison]::Ordinal)
            verified=@('ASCII after IME','Enter','Ctrl+End','newline selection','Undo/Redo/second Undo','save')
            not_run=@('phrase movement/reconversion','selection-replacement Undo branch')
            reason='The recorded source/history substeps passed; the complete post-IME movement and replacement branch remains untested.'
        }

        Invoke-EditorRecoveryE10

        $script:CurrentEditorRecoveryCaseId='E11'
        $focus=[MdliteRustAcceptanceNative]::CreateRecoveryFocusWindow()
        if ($focus -eq [IntPtr]::Zero) {
            Set-EditorRecoveryCase 'E11' 'BLOCKED' @{reason='Could not create empty owned focus window.'}
        } else {
            try {
                $runnerPid=[uint32][Diagnostics.Process]::GetCurrentProcess().Id
                if (-not [MdliteRustAcceptanceNative]::IsOwnedWindow($focus,$runnerPid)) { throw 'ENVIRONMENT_BLOCKED: focus fixture ownership failed.' }
                $source=[string](Read-Diagnostic).source
                Write-Host ('FOREGROUND_REQUIRED pid={0} hwnd=0x{1:X} kind=empty-focus-fixture' -f $runnerPid,$focus.ToInt64())
                [void][MdliteRustAcceptanceNative]::SetForegroundWindow($focus)
                $focusDeadline=[DateTime]::UtcNow.AddSeconds(15)
                while ([MdliteRustAcceptanceNative]::GetForegroundWindow() -ne $focus -and [DateTime]::UtcNow -lt $focusDeadline) {
                    Start-Sleep -Milliseconds 100
                }
                if ([MdliteRustAcceptanceNative]::GetForegroundWindow() -ne $focus) { throw 'ENVIRONMENT_BLOCKED: empty focus fixture did not become foreground.' }
                if ((Read-Diagnostic).source -cne $source) { throw 'PRODUCT_ASSERTION: source changed while Editor was unfocused.' }
                Set-OwnedForeground -Window $script:WindowHandle -WaitMs 15000
                $end=[Text.Encoding]::UTF8.GetByteCount($source)
                [void](Assert-EditorRecoveryDocument $source $end $false 'E11 source after focus return')
                Send-Ascii ' focus' $script:InputWindowHandle
                $after=$source+' focus'
                [void](Assert-EditorRecoveryDocument $after ([Text.Encoding]::UTF8.GetByteCount($after)) $true 'E11 input resumes')
                Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
                [void](Assert-EditorRecoveryDocument $source $end $false 'E11 Undo restores exact source')
                [void](Save-EditorRecoveryEvidence 'E11' 'focus-return')
                Set-EditorRecoveryCase 'E11' 'NOT_RUN' @{external_focus='PASS';input_resumed='PASS';panels_and_active_IME='NOT_RUN';reason='Panel and active-composition focus transitions remain untested.'}
            } finally { [void][MdliteRustAcceptanceNative]::DestroyWindow($focus) }
        }

# Source-pinned E12 Workspace display-settings flow. Insert before E13 only
# after Root validates the current owned control tree. It runs in the same
# two-tab Core process and never writes the Markdown document.
$script:CurrentEditorRecoveryCaseId='E12'
$e12CoreSource=[string]$script:EditorRecoveryExpectedSource
$e12Before=Switch-ToSource $e12CoreSource
if ([int]$e12Before.tabs -ne 2 -or [string]$e12Before.source -cne $e12CoreSource) {
    throw 'SETUP_FAILURE: E12 requires saved Core Workspace and exactly the original two tabs.'
}
$e12WorkspacePath=[string]$script:EditorRecoveryWorkspaceSettingsPath
$e12CommonPath=Join-Path $script:UiLocalAppData 'MDLite/settings.toml'
$e12ExpectedOriginalText='schema_version = 1'+$script:EditorRecoveryLf+'auto_save = false'+$script:EditorRecoveryLf
$e12ExpectedOriginalBytes=[Text.UTF8Encoding]::new($false).GetBytes($e12ExpectedOriginalText)
$e12InitialBytes=[IO.File]::ReadAllBytes($e12WorkspacePath)
$e12InitialHash=Get-EditorRecoveryBytesSha256 $e12InitialBytes
if ($e12InitialHash -cne (Get-EditorRecoveryBytesSha256 $e12ExpectedOriginalBytes)) {
    throw 'SETUP_FAILURE: E12 requires the exact two-line UTF-8/no-BOM/LF auto_save=false Workspace file.'
}
$e12CommonBeforeHash=Get-UiSafeFileHash $e12CommonPath
$e12DocumentId=[long]$e12Before.document_id
$e12SelectionBeforeJson=ConvertTo-Json -InputObject $e12Before.selection -Compress -Depth 12
$e12ProfileBefore=Get-EditorRecoveryProfileIdentity $e12Before.active_input_profile
if ($null -eq $e12ProfileBefore) { throw 'OBSERVER_FAILURE: E12 initial input-profile identity is unavailable.' }
$e12ProfileBeforeJson=ConvertTo-Json -InputObject $e12ProfileBefore -Compress -Depth 12
$e12AffinityBefore='UNOBSERVED'
$e12AffinityProperty=$e12Before.PSObject.Properties['selection_affinity']
if ($null -eq $e12AffinityProperty) { $e12AffinityProperty=$e12Before.selection.PSObject.Properties['affinity'] }
if ($null -eq $e12AffinityProperty) { throw 'OBSERVER_FAILURE: E12 selection affinity is unavailable before input.' }
$e12AffinityBefore=[string]$e12AffinityProperty.Value
$e12ImeActiveProperty=$e12Before.ime.PSObject.Properties['active']
if ($null -eq $e12ImeActiveProperty) { throw 'OBSERVER_FAILURE: E12 active-composition state is unavailable before opening SettingsGUI.' }
if ([bool]$e12ImeActiveProperty.Value) { throw 'ENVIRONMENT_BLOCKED: E12 cannot open SettingsGUI during active composition.' }
$e12InvariantBeforeJson=ConvertTo-Json -InputObject ([ordered]@{
    source=[string]$e12Before.source; document_id=[long]$e12Before.document_id
    revision=[int]$e12Before.revision; selection=$e12SelectionBeforeJson
    selection_affinity=$e12AffinityBefore; raw=[bool]$e12Before.raw
    dirty=[bool]$e12Before.dirty; undo_count=[int]$e12Before.undo_count
    redo_count=[int]$e12Before.redo_count; profile=$e12ProfileBeforeJson
}) -Compress -Depth 16
$e12InitialCaret=Get-EditorRecoveryCaretDipPoint $e12Before 'E12 initial Core caret'
Assert-EditorRecoveryViewGeometryMetadata $e12Before
$e12ExpectedTemporaryText='schema_version = 1'+$script:EditorRecoveryLf+
    'auto_save = false'+$script:EditorRecoveryLf+
    'font_size_pt = 20'+$script:EditorRecoveryLf+
    'theme = "light"'+$script:EditorRecoveryLf
$e12ExpectedTemporaryBytes=[Text.UTF8Encoding]::new($false).GetBytes($e12ExpectedTemporaryText)
$e12CaptureDirectory=Join-Path $script:UiRunDirectory 'cases/E12'
[void][IO.Directory]::CreateDirectory($e12CaptureDirectory)
$e12Captures=@()
$e12ControlEvidence=@()
$e12SettingsWindow=[IntPtr]::Zero

# Workspace session 1: stage only size/theme, apply, close, and capture the
# actual temporary rendering after App::show_settings reloads persisted state.
Invoke-UiControlById $script:WindowHandle 114
$e12SettingsWindow=Wait-UiThreadWindow 'MDLiteSettings' '' 3000
Assert-UiOwnedWindow $e12SettingsWindow
[uint32]$e12DialogPid=0
$e12DialogThread=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12SettingsWindow,[ref]$e12DialogPid)
if ($e12DialogThread -eq 0 -or $e12DialogPid -ne $script:ProcessId -or
    [MdliteRustAcceptanceNative]::WindowClass($e12SettingsWindow) -cne 'MDLiteSettings' -or
    -not [MdliteRustAcceptanceNative]::IsWindowVisible($e12SettingsWindow) -or
    -not [MdliteRustAcceptanceNative]::IsWindowEnabled($e12SettingsWindow)) {
    throw 'ENVIRONMENT_BLOCKED: E12 first SettingsGUI owner/class/visibility/enabled identity changed.'
}
$e12Scope=Get-UiControlById -Root $e12SettingsWindow -ControlId 200 -Class 'COMBOBOX' -RequireVisible -RequireEnabled
$e12AutoSave=Get-UiControlById -Root $e12SettingsWindow -ControlId 201 -Class 'COMBOBOX' -RequireVisible -RequireEnabled
$e12Theme=Get-UiControlById -Root $e12SettingsWindow -ControlId 203 -Class 'COMBOBOX' -RequireVisible -RequireEnabled
$e12FontSize=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
$e12Apply=Get-UiControlById -Root $e12SettingsWindow -ControlId 210 -Class 'BUTTON' -RequireVisible
$e12Cancel=Get-UiControlById -Root $e12SettingsWindow -ControlId 211 -Class 'BUTTON' -RequireVisible -RequireEnabled
foreach ($control in @($e12Scope,$e12AutoSave,$e12Theme,$e12FontSize,$e12Apply,$e12Cancel)) {
    [uint32]$e12ControlPid=0
    $e12ControlThread=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($control.Handle,[ref]$e12ControlPid)
    if ($e12ControlPid -ne $script:ProcessId -or $e12ControlThread -ne $e12DialogThread) {
        throw 'ENVIRONMENT_BLOCKED: E12 first SettingsGUI control left its exact owned dialog thread.'
    }
}
$e12ControlEvidence+=@(
    foreach ($control in @($e12Scope,$e12AutoSave,$e12Theme,$e12FontSize,$e12Apply,$e12Cancel)) {
        [ordered]@{
            phase='temporary-session'; hwnd='0x{0:X}' -f $control.Handle.ToInt64(); id=[int]$control.Id
            class=[string]$control.Class; visible=[bool]$control.Visible
            enabled=[bool]$control.Enabled; rect=@($control.Rect)
            pid=[int]$script:ProcessId; thread=[long]$e12DialogThread
        }
    }
)
$e12CommonScope=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 200 0 'ユーザー共通'
Invoke-UiControlClick $e12SettingsWindow $e12Scope
Send-Key $e12SettingsWindow 0x24
Send-Key $e12SettingsWindow 0x28
$e12ScopeReadbackA=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 200 1 'Workspace'
$e12AutoSaveBeforeA=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 201 2 'false'
$e12ThemeBeforeA=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 203 0 'inherit'
$e12FontBeforeA=[MdliteRustAcceptanceNative]::OwnedControlText($e12FontSize.Handle,[uint32]$script:ProcessId)
if ($null -eq $e12FontBeforeA -or [string]$e12FontBeforeA -cne '') {
    throw 'PRODUCT_ASSERTION: E12 initial Workspace font-size Edit is not blank/inherited.'
}
$e12FontRecheckA=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
if ($e12FontRecheckA.Handle -ne $e12FontSize.Handle) {
    throw 'ENVIRONMENT_BLOCKED: E12 first font-size Edit identity changed during preflight.'
}
$e12Captures+=Join-Path $e12CaptureDirectory '001-workspace-inherited-controls.png'
Capture-UiOwnedWindow $e12SettingsWindow $e12Captures[-1]

# Normal owned field keys change only the SettingsGUI EDIT205.
Invoke-UiControlClick $e12SettingsWindow $e12FontSize
$e12FontFocusA=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
if ($e12FontFocusA.Handle -ne $e12FontSize.Handle) {
    throw 'ENVIRONMENT_BLOCKED: E12 font-size Edit205 identity changed after the owned focus click.'
}
[uint32]$e12FontFocusPidA=0
$e12FontFocusThreadA=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12FontFocusA.Handle,[ref]$e12FontFocusPidA)
if ($e12FontFocusPidA -ne $script:ProcessId -or $e12FontFocusThreadA -ne $e12DialogThread) {
    throw 'ENVIRONMENT_BLOCKED: E12 font-size Edit205 control left the SettingsGUI owner thread.'
}
Set-OwnedForeground -Window $e12SettingsWindow
$e12ForegroundA=[MdliteRustAcceptanceNative]::GetForegroundWindow()
[uint32]$e12ForegroundPidA=0
if ($e12ForegroundA -ne [IntPtr]::Zero) {
    [void][MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12ForegroundA,[ref]$e12ForegroundPidA)
}
if ($e12ForegroundA -ne $e12SettingsWindow -or $e12ForegroundPidA -ne $script:ProcessId) {
    throw 'ENVIRONMENT_BLOCKED: E12 font-size normal keys are not routed through the owned SettingsGUI.'
}
[uint32]$e12MainThreadPidA=0
$e12MainThreadA=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($script:WindowHandle,[ref]$e12MainThreadPidA)
if ($e12MainThreadPidA -ne $script:ProcessId -or $e12MainThreadA -ne $e12DialogThread) {
    throw 'ENVIRONMENT_BLOCKED: E12 SettingsGUI Edit205 does not share the registered Main GUI thread.'
}
$e12FontFocusInfoA=[MdliteRustAcceptanceNative]::ObserveOwnedEditorFocus(
    $script:WindowHandle,$script:WindowHandle,[IntPtr]::Zero,[uint32]$script:ProcessId)
if (-not $e12FontFocusInfoA.registered_editor_window -or
    $e12FontFocusInfoA.window_class -cne 'MDLiteRustEditor' -or
    -not $e12FontFocusInfoA.gui_thread_info_available) {
    throw 'OBSERVER_FAILURE: E12 registered Main GUIThreadInfo is unavailable before Edit205 input.'
}
$e12FontFocusHandleTextA='0x{0:X}' -f $e12FontFocusA.Handle.ToInt64()
$e12SettingsHandleTextA='0x{0:X}' -f $e12SettingsWindow.ToInt64()
if ($e12FontFocusInfoA.focus_hwnd -cne $e12FontFocusHandleTextA -or
    $e12FontFocusInfoA.active_hwnd -cne $e12SettingsHandleTextA) {
    throw 'ENVIRONMENT_BLOCKED: E12 raw GUIThreadInfo focus/active HWND does not identify owned Edit205/SettingsGUI before input.'
}
$e12Edit205FocusEvidenceA=[ordered]@{
    status='PASS'; process_id=[uint32]$script:ProcessId
    main_thread_pid=[uint32]$e12MainThreadPidA; settings_pid=[uint32]$e12DialogPid
    edit205_pid=[uint32]$e12FontFocusPidA
    registered_main=$e12FontFocusInfoA.registered_editor_window
    main_class=$e12FontFocusInfoA.window_class; gui_thread_info_available=$e12FontFocusInfoA.gui_thread_info_available
    active_hwnd=$e12FontFocusInfoA.active_hwnd; focus_hwnd=$e12FontFocusInfoA.focus_hwnd
    settings_hwnd=$e12SettingsHandleTextA; edit205_hwnd=$e12FontFocusHandleTextA
    settings_foreground_hwnd=('0x{0:X}' -f $e12ForegroundA.ToInt64())
    main_thread_id=[uint32]$e12MainThreadA; edit_thread_id=[uint32]$e12FontFocusThreadA
}
Send-Key $e12SettingsWindow 0x41 ([uint16[]]@(0x11))
Send-Key $e12SettingsWindow 0x08
Send-Ascii '20' $e12SettingsWindow
$e12FontReadbackDeadlineA=[DateTime]::UtcNow.AddSeconds(3)
$e12FontReadbackMatchedA=$false
$e12FontStagedTextA=$null
do {
    $e12FontRemainingA=[int][Math]::Floor(($e12FontReadbackDeadlineA-[DateTime]::UtcNow).TotalMilliseconds)
    if ($e12FontRemainingA -lt 1000) { break }
    $e12FontReadbackControlA=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
    if ($e12FontReadbackControlA.Handle -ne $e12FontFocusA.Handle) {
        throw 'ENVIRONMENT_BLOCKED: E12 font-size Edit205 identity changed during bounded text readback.'
    }
    [uint32]$e12FontReadbackPidA=0
    $e12FontReadbackThreadA=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12FontReadbackControlA.Handle,[ref]$e12FontReadbackPidA)
    if ($e12FontReadbackPidA -ne $script:ProcessId -or $e12FontReadbackThreadA -ne $e12DialogThread) {
        throw 'ENVIRONMENT_BLOCKED: E12 font-size Edit205 ownership changed during bounded text readback.'
    }
    $e12FontStagedTextA=[MdliteRustAcceptanceNative]::OwnedControlText($e12FontReadbackControlA.Handle,[uint32]$script:ProcessId)
    if ($null -eq $e12FontStagedTextA) { throw 'OBSERVER_FAILURE: E12 font-size Edit205 WM_GETTEXT returned null.' }
    if ([string]::Equals([string]$e12FontStagedTextA,'20',[StringComparison]::Ordinal)) {
        $e12FontReadbackMatchedA=$true
        break
    }
    if (([DateTime]::UtcNow-$e12FontReadbackDeadlineA).TotalMilliseconds -lt -50) {
        Start-Sleep -Milliseconds 25
    }
} while ([DateTime]::UtcNow -lt $e12FontReadbackDeadlineA)
if (-not $e12FontReadbackMatchedA) {
    throw 'SETUP_FAILURE: normal ASCII delivery to SettingsGUI Edit205 did not produce exact text 20 within the 3s step budget.'
}
$e12FontStagedA=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
if ($e12FontStagedA.Handle -ne $e12FontSize.Handle -or
    $e12FontStagedA.Handle -ne $e12FontFocusA.Handle -or
    [string]$e12FontStagedTextA -cne '20') {
    throw 'PRODUCT_ASSERTION: E12 font-size Edit205 did not stage exact valid value 20.'
}
Invoke-UiControlClick $e12SettingsWindow $e12Theme
Send-Key $e12SettingsWindow 0x24
Send-Key $e12SettingsWindow 0x28
Send-Key $e12SettingsWindow 0x28
$e12ThemeStagedA=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 203 2 'light'
$e12AutoSaveStagedA=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 201 2 'false'
$e12Captures+=Join-Path $e12CaptureDirectory '002-workspace-light20-pending-apply.png'
Capture-UiOwnedWindow $e12SettingsWindow $e12Captures[-1]
Invoke-UiDialogButton $e12SettingsWindow 210
try {
    $e12NoticeA=Wait-UiThreadWindow '#32770' '設定' 3000
} catch {
    $e12ApplyWaitErrorA=$_.Exception
    $e12FailureNoticeA=$null
    $e12FailureTitleA=$null
    foreach ($title in @('設定を保存できません — 入力保持','設定を保存できません')) {
        try { $e12FailureNoticeA=Wait-UiThreadWindow '#32770' $title 250; $e12FailureTitleA=$title; break } catch { }
    }
    if ($null -ne $e12FailureNoticeA) {
        Assert-UiOwnedWindow $e12FailureNoticeA
        [uint32]$e12FailurePidA=0
        $e12FailureThreadA=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12FailureNoticeA,[ref]$e12FailurePidA)
        if ($e12FailurePidA -ne $script:ProcessId -or $e12FailureThreadA -ne $e12DialogThread) {
            throw 'ENVIRONMENT_BLOCKED: E12 temporary Apply error notice is outside the owned SettingsGUI thread.'
        }
        throw [System.InvalidOperationException]::new(
            ('PRODUCT_ASSERTION: E12 temporary Apply error notice "{0}" is retained; stop E13 and keep the owned SettingsGUI available.' -f $e12FailureTitleA),
            $e12ApplyWaitErrorA)
    }
    throw $e12ApplyWaitErrorA
}
Assert-UiOwnedWindow $e12NoticeA
[uint32]$e12NoticePidA=0
$e12NoticeThreadA=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12NoticeA,[ref]$e12NoticePidA)
if ($e12NoticePidA -ne $script:ProcessId -or $e12NoticeThreadA -ne $e12DialogThread) {
    throw 'ENVIRONMENT_BLOCKED: E12 first Apply notice is not owned by the exact SettingsGUI process.'
}
Invoke-UiDialogButton $e12NoticeA 1
Wait-UiWindowGone $e12NoticeA
if (-not [MdliteRustAcceptanceNative]::IsWindow($e12SettingsWindow)) {
    throw 'PRODUCT_ASSERTION: E12 Apply unexpectedly closed the SettingsGUI.'
}
$e12TemporaryBytesAfterApply=[IO.File]::ReadAllBytes($e12WorkspacePath)
$e12TemporaryHashAfterApply=Get-EditorRecoveryBytesSha256 $e12TemporaryBytesAfterApply
if ($e12TemporaryHashAfterApply -cne (Get-EditorRecoveryBytesSha256 $e12ExpectedTemporaryBytes)) {
    throw 'PRODUCT_ASSERTION: E12 temporary Apply bytes differ from exact sorted Workspace patch.'
}
if ((Get-UiSafeFileHash $e12CommonPath) -cne $e12CommonBeforeHash) {
    throw 'PRODUCT_ASSERTION: E12 Workspace Apply changed the isolated Common settings file.'
}
$e12AutoSaveAfterApplyA=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 201 2 'false'
$e12ThemeAfterApplyA=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 203 2 'light'
$e12FontAfterApplyA=[MdliteRustAcceptanceNative]::OwnedControlText($e12FontStagedA.Handle,[uint32]$script:ProcessId)
if ([string]$e12FontAfterApplyA -cne '20') {
    throw 'PRODUCT_ASSERTION: E12 font-size Edit205 changed after successful Apply.'
}
$e12Captures+=Join-Path $e12CaptureDirectory '003-settings-after-temporary-apply.png'
Capture-UiOwnedWindow $e12SettingsWindow $e12Captures[-1]
# Cancel closes a clean applied form; it does not roll back the successful Apply.
Invoke-UiDialogButton $e12SettingsWindow 211
Wait-UiWindowGone $e12SettingsWindow
if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e12WorkspacePath))) -cne
    (Get-EditorRecoveryBytesSha256 $e12ExpectedTemporaryBytes) -or
    (Get-UiSafeFileHash $e12CommonPath) -cne $e12CommonBeforeHash) {
    throw 'PRODUCT_ASSERTION: E12 Cancel after Apply changed temporary Workspace bytes or Common settings.'
}
Set-OwnedForeground -Window $script:WindowHandle
$e12TempStateA=Wait-EditorRecoveryStableState
Assert-EditorRecoveryViewGeometryMetadata $e12TempStateA
$e12TempCaretA=Get-EditorRecoveryCaretDipPoint $e12TempStateA 'E12 applied font20/theme-light viewport'
$e12TempStateB=Wait-EditorRecoveryStableState
Assert-EditorRecoveryViewGeometryMetadata $e12TempStateB
$e12TempCaretB=Get-EditorRecoveryCaretDipPoint $e12TempStateB 'E12 stable applied font20/theme-light viewport'
$e12TempGeometryAJson=ConvertTo-Json -InputObject ([ordered]@{
    caret=$e12TempStateA.caret; native_caret=$e12TempStateA.native_caret
    scroll=$e12TempStateA.scroll; views=$e12TempStateA.views; point=$e12TempCaretA
}) -Compress -Depth 16
$e12TempGeometryBJson=ConvertTo-Json -InputObject ([ordered]@{
    caret=$e12TempStateB.caret; native_caret=$e12TempStateB.native_caret
    scroll=$e12TempStateB.scroll; views=$e12TempStateB.views; point=$e12TempCaretB
}) -Compress -Depth 16
if ($e12TempGeometryAJson -cne $e12TempGeometryBJson) {
    throw 'ENVIRONMENT_BLOCKED: E12 temporary font/theme caret viewport geometry did not stabilize.'
}
$e12TempState=Assert-EditorRecoverySelection $e12CoreSource $e12DocumentId `
    ([int]$e12Before.selection.anchor) ([int]$e12Before.selection.head) `
    ([bool]$e12Before.dirty) ([int]$e12Before.undo_count) ([int]$e12Before.redo_count) `
    'E12 temporary font/theme preserves Source/Rope indices and history'
$e12ProfileTemp=Get-EditorRecoveryProfileIdentity $e12TempState.active_input_profile
$e12ProfileTempJson=ConvertTo-Json -InputObject $e12ProfileTemp -Compress -Depth 12
$e12AffinityTemp='UNOBSERVED'
$e12AffinityTempProperty=$e12TempState.PSObject.Properties['selection_affinity']
if ($null -eq $e12AffinityTempProperty) { $e12AffinityTempProperty=$e12TempState.selection.PSObject.Properties['affinity'] }
if ($null -ne $e12AffinityTempProperty) { $e12AffinityTemp=[string]$e12AffinityTempProperty.Value }
$e12InvariantTempJson=ConvertTo-Json -InputObject ([ordered]@{
    source=[string]$e12TempState.source; document_id=[long]$e12TempState.document_id
    revision=[int]$e12TempState.revision
    selection=(ConvertTo-Json -InputObject $e12TempState.selection -Compress -Depth 12)
    selection_affinity=$e12AffinityTemp; raw=[bool]$e12TempState.raw
    dirty=[bool]$e12TempState.dirty; undo_count=[int]$e12TempState.undo_count
    redo_count=[int]$e12TempState.redo_count; profile=$e12ProfileTempJson
}) -Compress -Depth 16
if ($e12InvariantTempJson -cne $e12InvariantBeforeJson -or $e12AffinityTemp -cne $e12AffinityBefore -or
    $e12ProfileTempJson -cne $e12ProfileBeforeJson) {
    throw 'PRODUCT_ASSERTION: E12 temporary settings changed Core source, document, selection/affinity, dirty/history, or profile.'
}
$e12Captures+=Join-Path $e12CaptureDirectory '004-main-temporary-font20-theme-light.png'
Capture-UiOwnedWindow $script:WindowHandle $e12Captures[-1]

# Workspace session 2: confirm the saved temporary values, inherit only theme
# and font size, apply, then verify exact original bytes before E13.
$e12SettingsWindow=[IntPtr]::Zero
if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e12WorkspacePath))) -cne
    (Get-EditorRecoveryBytesSha256 $e12ExpectedTemporaryBytes)) {
    throw 'SETUP_FAILURE: E12 restore session requires the exact temporary Workspace file.'
}
Invoke-UiControlById $script:WindowHandle 114
$e12SettingsWindow=Wait-UiThreadWindow 'MDLiteSettings' '' 3000
Assert-UiOwnedWindow $e12SettingsWindow
[uint32]$e12DialogPidB=0
$e12DialogThreadB=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12SettingsWindow,[ref]$e12DialogPidB)
if ($e12DialogThreadB -eq 0 -or $e12DialogPidB -ne $script:ProcessId -or
    [MdliteRustAcceptanceNative]::WindowClass($e12SettingsWindow) -cne 'MDLiteSettings' -or
    -not [MdliteRustAcceptanceNative]::IsWindowVisible($e12SettingsWindow) -or
    -not [MdliteRustAcceptanceNative]::IsWindowEnabled($e12SettingsWindow)) {
    throw 'ENVIRONMENT_BLOCKED: E12 restore SettingsGUI owner/class/visibility/enabled identity changed.'
}
$e12ScopeB=Get-UiControlById -Root $e12SettingsWindow -ControlId 200 -Class 'COMBOBOX' -RequireVisible -RequireEnabled
$e12AutoSaveB=Get-UiControlById -Root $e12SettingsWindow -ControlId 201 -Class 'COMBOBOX' -RequireVisible -RequireEnabled
$e12ThemeB=Get-UiControlById -Root $e12SettingsWindow -ControlId 203 -Class 'COMBOBOX' -RequireVisible -RequireEnabled
$e12FontSizeB=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
$e12ApplyB=Get-UiControlById -Root $e12SettingsWindow -ControlId 210 -Class 'BUTTON' -RequireVisible
$e12CancelB=Get-UiControlById -Root $e12SettingsWindow -ControlId 211 -Class 'BUTTON' -RequireVisible -RequireEnabled
foreach ($control in @($e12ScopeB,$e12AutoSaveB,$e12ThemeB,$e12FontSizeB,$e12ApplyB,$e12CancelB)) {
    [uint32]$e12ControlPidB=0
    $e12ControlThreadB=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($control.Handle,[ref]$e12ControlPidB)
    if ($e12ControlPidB -ne $script:ProcessId -or $e12ControlThreadB -ne $e12DialogThreadB) {
        throw 'ENVIRONMENT_BLOCKED: E12 restore control left its exact owned SettingsGUI thread.'
    }
}
$e12ControlEvidence+=@(
    foreach ($control in @($e12ScopeB,$e12AutoSaveB,$e12ThemeB,$e12FontSizeB,$e12ApplyB,$e12CancelB)) {
        [ordered]@{
            phase='restore-session'; hwnd='0x{0:X}' -f $control.Handle.ToInt64()
            id=[int]$control.Id; class=[string]$control.Class
            visible=[bool]$control.Visible; enabled=[bool]$control.Enabled
            rect=@($control.Rect); pid=[int]$script:ProcessId; thread=[long]$e12DialogThreadB
        }
    }
)
$e12CommonScopeB=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 200 0 'ユーザー共通'
Invoke-UiControlClick $e12SettingsWindow $e12ScopeB
Send-Key $e12SettingsWindow 0x24
Send-Key $e12SettingsWindow 0x28
$e12ScopeReadbackB=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 200 1 'Workspace'
$e12AutoSaveBeforeB=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 201 2 'false'
$e12ThemeBeforeB=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 203 2 'light'
$e12FontBeforeB=[MdliteRustAcceptanceNative]::OwnedControlText($e12FontSizeB.Handle,[uint32]$script:ProcessId)
if ([string]$e12FontBeforeB -cne '20') {
    throw 'PRODUCT_ASSERTION: E12 restore preflight does not read the applied font-size value 20.'
}
$e12FontRecheckB=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
if ($e12FontRecheckB.Handle -ne $e12FontSizeB.Handle) {
    throw 'ENVIRONMENT_BLOCKED: E12 restore font-size Edit identity changed during preflight.'
}
$e12Captures+=Join-Path $e12CaptureDirectory '005-workspace-light20-before-restore.png'
Capture-UiOwnedWindow $e12SettingsWindow $e12Captures[-1]

Invoke-UiControlClick $e12SettingsWindow $e12ThemeB
Send-Key $e12SettingsWindow 0x24
$e12ThemeInheritB=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 203 0 'inherit'
Invoke-UiControlClick $e12SettingsWindow $e12FontSizeB
$e12FontClearFocusB=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
if ($e12FontClearFocusB.Handle -ne $e12FontSizeB.Handle) {
    throw 'ENVIRONMENT_BLOCKED: E12 restore font-size Edit205 identity changed after the owned focus click.'
}
[uint32]$e12FontClearPidB=0
$e12FontClearThreadB=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12FontClearFocusB.Handle,[ref]$e12FontClearPidB)
if ($e12FontClearPidB -ne $script:ProcessId -or $e12FontClearThreadB -ne $e12DialogThreadB) {
    throw 'ENVIRONMENT_BLOCKED: E12 restore font-size Edit205 control left the SettingsGUI owner thread.'
}
Set-OwnedForeground -Window $e12SettingsWindow
$e12ForegroundB=[MdliteRustAcceptanceNative]::GetForegroundWindow()
[uint32]$e12ForegroundPidB=0
if ($e12ForegroundB -ne [IntPtr]::Zero) {
    [void][MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12ForegroundB,[ref]$e12ForegroundPidB)
}
if ($e12ForegroundB -ne $e12SettingsWindow -or $e12ForegroundPidB -ne $script:ProcessId) {
    throw 'ENVIRONMENT_BLOCKED: E12 restore font-size normal keys are not routed through the owned SettingsGUI.'
}
[uint32]$e12MainThreadPidB=0
$e12MainThreadB=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($script:WindowHandle,[ref]$e12MainThreadPidB)
if ($e12MainThreadPidB -ne $script:ProcessId -or $e12MainThreadB -ne $e12DialogThreadB) {
    throw 'ENVIRONMENT_BLOCKED: E12 restore SettingsGUI Edit205 does not share the registered Main GUI thread.'
}
$e12FontFocusInfoB=[MdliteRustAcceptanceNative]::ObserveOwnedEditorFocus(
    $script:WindowHandle,$script:WindowHandle,[IntPtr]::Zero,[uint32]$script:ProcessId)
if (-not $e12FontFocusInfoB.registered_editor_window -or
    $e12FontFocusInfoB.window_class -cne 'MDLiteRustEditor' -or
    -not $e12FontFocusInfoB.gui_thread_info_available) {
    throw 'OBSERVER_FAILURE: E12 registered Main GUIThreadInfo is unavailable before restore Edit205 input.'
}
$e12FontClearHandleTextB='0x{0:X}' -f $e12FontClearFocusB.Handle.ToInt64()
$e12SettingsHandleTextB='0x{0:X}' -f $e12SettingsWindow.ToInt64()
if ($e12FontFocusInfoB.focus_hwnd -cne $e12FontClearHandleTextB -or
    $e12FontFocusInfoB.active_hwnd -cne $e12SettingsHandleTextB) {
    throw 'ENVIRONMENT_BLOCKED: E12 raw GUIThreadInfo focus/active HWND does not identify owned restore Edit205/SettingsGUI before input.'
}
$e12Edit205FocusEvidenceB=[ordered]@{
    status='PASS'; process_id=[uint32]$script:ProcessId
    main_thread_pid=[uint32]$e12MainThreadPidB; settings_pid=[uint32]$e12DialogPidB
    edit205_pid=[uint32]$e12FontClearPidB
    registered_main=$e12FontFocusInfoB.registered_editor_window
    main_class=$e12FontFocusInfoB.window_class; gui_thread_info_available=$e12FontFocusInfoB.gui_thread_info_available
    active_hwnd=$e12FontFocusInfoB.active_hwnd; focus_hwnd=$e12FontFocusInfoB.focus_hwnd
    settings_hwnd=$e12SettingsHandleTextB; edit205_hwnd=$e12FontClearHandleTextB
    settings_foreground_hwnd=('0x{0:X}' -f $e12ForegroundB.ToInt64())
    main_thread_id=[uint32]$e12MainThreadB; edit_thread_id=[uint32]$e12FontClearThreadB
}
Send-Key $e12SettingsWindow 0x41 ([uint16[]]@(0x11))
Send-Key $e12SettingsWindow 0x08
$e12FontReadbackDeadlineB=[DateTime]::UtcNow.AddSeconds(3)
$e12FontReadbackMatchedB=$false
$e12FontInheritedB=$null
do {
    $e12FontRemainingB=[int][Math]::Floor(($e12FontReadbackDeadlineB-[DateTime]::UtcNow).TotalMilliseconds)
    if ($e12FontRemainingB -lt 1000) { break }
    $e12FontInheritedControlB=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
    if ($e12FontInheritedControlB.Handle -ne $e12FontSizeB.Handle) {
        throw 'ENVIRONMENT_BLOCKED: E12 restore font-size Edit205 identity changed during bounded text readback.'
    }
    [uint32]$e12FontReadbackPidB=0
    $e12FontReadbackThreadB=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12FontInheritedControlB.Handle,[ref]$e12FontReadbackPidB)
    if ($e12FontReadbackPidB -ne $script:ProcessId -or $e12FontReadbackThreadB -ne $e12DialogThreadB) {
        throw 'ENVIRONMENT_BLOCKED: E12 restore font-size Edit205 ownership changed during bounded text readback.'
    }
    $e12FontInheritedB=[MdliteRustAcceptanceNative]::OwnedControlText($e12FontInheritedControlB.Handle,[uint32]$script:ProcessId)
    if ($null -eq $e12FontInheritedB) { throw 'OBSERVER_FAILURE: E12 restored font-size Edit205 WM_GETTEXT returned null.' }
    if ([string]::Equals([string]$e12FontInheritedB,'',[StringComparison]::Ordinal)) {
        $e12FontReadbackMatchedB=$true
        break
    }
    if (([DateTime]::UtcNow-$e12FontReadbackDeadlineB).TotalMilliseconds -lt -50) {
        Start-Sleep -Milliseconds 25
    }
} while ([DateTime]::UtcNow -lt $e12FontReadbackDeadlineB)
if (-not $e12FontReadbackMatchedB) {
    throw 'SETUP_FAILURE: normal Ctrl+A/Backspace delivery did not clear SettingsGUI Edit205 within the 3s step budget.'
}
$e12FontInheritedControlB=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
if ($e12FontInheritedControlB.Handle -ne $e12FontSizeB.Handle -or
    [string]$e12FontInheritedB -cne '') {
    throw 'PRODUCT_ASSERTION: E12 font-size Edit205 did not stage blank/inherit.'
}
$e12AutoSaveInheritedB=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 201 2 'false'
$e12Captures+=Join-Path $e12CaptureDirectory '006-workspace-inherit-fontblank-pending-apply.png'
Capture-UiOwnedWindow $e12SettingsWindow $e12Captures[-1]
Invoke-UiDialogButton $e12SettingsWindow 210
try {
    $e12NoticeB=Wait-UiThreadWindow '#32770' '設定' 3000
} catch {
    $e12ApplyWaitErrorB=$_.Exception
    $e12FailureNoticeB=$null
    $e12FailureTitleB=$null
    foreach ($title in @('設定を保存できません — 入力保持','設定を保存できません')) {
        try { $e12FailureNoticeB=Wait-UiThreadWindow '#32770' $title 250; $e12FailureTitleB=$title; break } catch { }
    }
    if ($null -ne $e12FailureNoticeB) {
        Assert-UiOwnedWindow $e12FailureNoticeB
        [uint32]$e12FailurePidB=0
        $e12FailureThreadB=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12FailureNoticeB,[ref]$e12FailurePidB)
        if ($e12FailurePidB -ne $script:ProcessId -or $e12FailureThreadB -ne $e12DialogThreadB) {
            throw 'ENVIRONMENT_BLOCKED: E12 restore Apply error notice is outside the owned SettingsGUI thread.'
        }
        throw [System.InvalidOperationException]::new(
            ('PRODUCT_ASSERTION: E12 restore Apply error notice "{0}" is retained; keep Workspace ownership and do not hand-write file bytes.' -f $e12FailureTitleB),
            $e12ApplyWaitErrorB)
    }
    throw $e12ApplyWaitErrorB
}
Assert-UiOwnedWindow $e12NoticeB
[uint32]$e12NoticePidB=0
$e12NoticeThreadB=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e12NoticeB,[ref]$e12NoticePidB)
if ($e12NoticePidB -ne $script:ProcessId -or $e12NoticeThreadB -ne $e12DialogThreadB) {
    throw 'ENVIRONMENT_BLOCKED: E12 restore Apply notice is not owned by the exact SettingsGUI process.'
}
Invoke-UiDialogButton $e12NoticeB 1
Wait-UiWindowGone $e12NoticeB
if (-not [MdliteRustAcceptanceNative]::IsWindow($e12SettingsWindow)) {
    throw 'PRODUCT_ASSERTION: E12 restore Apply unexpectedly closed the SettingsGUI.'
}
$e12WorkspaceRestoredBytes=[IO.File]::ReadAllBytes($e12WorkspacePath)
$e12WorkspaceRestoredHash=Get-EditorRecoveryBytesSha256 $e12WorkspaceRestoredBytes
if ($e12WorkspaceRestoredHash -cne $e12InitialHash -or
    $e12WorkspaceRestoredHash -cne (Get-EditorRecoveryBytesSha256 $e12ExpectedOriginalBytes) -or
    (Get-UiSafeFileHash $e12CommonPath) -cne $e12CommonBeforeHash) {
    throw 'PRODUCT_ASSERTION: E12 inherit Apply failed exact Workspace-byte restoration or changed Common settings; stop before E13.'
}
$e12AutoSaveAfterRestore=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 201 2 'false'
$e12ThemeAfterRestore=Get-UiOwnedSettingsComboSelection $e12SettingsWindow 203 0 'inherit'
$e12FontRestoredControl=Get-UiControlById -Root $e12SettingsWindow -ControlId 205 -Class 'EDIT' -RequireVisible -RequireEnabled
if ($e12FontRestoredControl.Handle -ne $e12FontSizeB.Handle) {
    throw 'ENVIRONMENT_BLOCKED: E12 restored font-size Edit identity changed after Apply.'
}
$e12FontAfterRestore=[MdliteRustAcceptanceNative]::OwnedControlText($e12FontRestoredControl.Handle,[uint32]$script:ProcessId)
if ($null -eq $e12FontAfterRestore -or [string]$e12FontAfterRestore -cne '') {
    throw 'PRODUCT_ASSERTION: E12 restored font-size Edit205 is not blank/inherited.'
}
$e12Captures+=Join-Path $e12CaptureDirectory '007-settings-after-exact-restore.png'
Capture-UiOwnedWindow $e12SettingsWindow $e12Captures[-1]
Invoke-UiDialogButton $e12SettingsWindow 211
Wait-UiWindowGone $e12SettingsWindow
if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e12WorkspacePath))) -cne $e12InitialHash -or
    (Get-UiSafeFileHash $e12CommonPath) -cne $e12CommonBeforeHash) {
    throw 'PRODUCT_ASSERTION: E12 Cancel after exact restore changed Workspace bytes or Common settings.'
}
Set-OwnedForeground -Window $script:WindowHandle
$e12RestoreStateA=Wait-EditorRecoveryStableState
Assert-EditorRecoveryViewGeometryMetadata $e12RestoreStateA
$e12RestoreCaretA=Get-EditorRecoveryCaretDipPoint $e12RestoreStateA 'E12 restored inherited font/theme viewport'
$e12RestoreStateB=Wait-EditorRecoveryStableState
Assert-EditorRecoveryViewGeometryMetadata $e12RestoreStateB
$e12RestoreCaretB=Get-EditorRecoveryCaretDipPoint $e12RestoreStateB 'E12 stable restored inherited viewport'
$e12RestoreGeometryAJson=ConvertTo-Json -InputObject ([ordered]@{
    caret=$e12RestoreStateA.caret; native_caret=$e12RestoreStateA.native_caret
    scroll=$e12RestoreStateA.scroll; views=$e12RestoreStateA.views; point=$e12RestoreCaretA
}) -Compress -Depth 16
$e12RestoreGeometryBJson=ConvertTo-Json -InputObject ([ordered]@{
    caret=$e12RestoreStateB.caret; native_caret=$e12RestoreStateB.native_caret
    scroll=$e12RestoreStateB.scroll; views=$e12RestoreStateB.views; point=$e12RestoreCaretB
}) -Compress -Depth 16
if ($e12RestoreGeometryAJson -cne $e12RestoreGeometryBJson) {
    throw 'ENVIRONMENT_BLOCKED: E12 restored inherited caret viewport geometry did not stabilize.'
}
$e12RestoredState=Assert-EditorRecoverySelection $e12CoreSource $e12DocumentId `
    ([int]$e12Before.selection.anchor) ([int]$e12Before.selection.head) `
    ([bool]$e12Before.dirty) ([int]$e12Before.undo_count) ([int]$e12Before.redo_count) `
    'E12 restored font/theme preserves Source/Rope indices and history'
$e12ProfileRestored=Get-EditorRecoveryProfileIdentity $e12RestoredState.active_input_profile
$e12ProfileRestoredJson=ConvertTo-Json -InputObject $e12ProfileRestored -Compress -Depth 12
$e12AffinityRestored='UNOBSERVED'
$e12AffinityRestoredProperty=$e12RestoredState.PSObject.Properties['selection_affinity']
if ($null -eq $e12AffinityRestoredProperty) { $e12AffinityRestoredProperty=$e12RestoredState.selection.PSObject.Properties['affinity'] }
if ($null -ne $e12AffinityRestoredProperty) { $e12AffinityRestored=[string]$e12AffinityRestoredProperty.Value }
$e12InvariantRestoredJson=ConvertTo-Json -InputObject ([ordered]@{
    source=[string]$e12RestoredState.source; document_id=[long]$e12RestoredState.document_id
    revision=[int]$e12RestoredState.revision
    selection=(ConvertTo-Json -InputObject $e12RestoredState.selection -Compress -Depth 12)
    selection_affinity=$e12AffinityRestored; raw=[bool]$e12RestoredState.raw
    dirty=[bool]$e12RestoredState.dirty; undo_count=[int]$e12RestoredState.undo_count
    redo_count=[int]$e12RestoredState.redo_count; profile=$e12ProfileRestoredJson
}) -Compress -Depth 16
if ($e12InvariantRestoredJson -cne $e12InvariantBeforeJson -or
    $e12AffinityRestored -cne $e12AffinityBefore -or
    $e12ProfileRestoredJson -cne $e12ProfileBeforeJson) {
    throw 'PRODUCT_ASSERTION: E12 restoration changed Core source, document, selection/affinity, dirty/history, or profile.'
}
$e12Captures+=Join-Path $e12CaptureDirectory '008-main-restored-inherited-font-theme.png'
Capture-UiOwnedWindow $script:WindowHandle $e12Captures[-1]
$e12CommonAfterHash=Get-UiSafeFileHash $e12CommonPath
if ($e12CommonAfterHash -cne $e12CommonBeforeHash) {
    throw 'PRODUCT_ASSERTION: E12 final Main capture changed the isolated Common settings hash/absence.'
}
Set-EditorRecoveryCase 'E12' 'NOT_RUN' @{
    verified=@('Workspace scope with source-pinned IDs/classes and exact current owner/tree preflight',
        'registered Main GUIThreadInfo raw focus/active HWNDs identified owned Edit205/SettingsGUI before both key sequences',
        'font_size_pt 20 and theme light applied only in Workspace; auto_save false unchanged',
        'second Workspace Apply inherited only font_size_pt/theme and restored exact original bytes',
        'Common settings hash/absence and Core Source/Rope selection/history/profile unchanged',
        'temporary and restored caret/view geometry stable within the owned editor viewport')
    workspace_control_ids=@{scope=200;auto_save=201;theme=203;font_size_pt=205;apply=210;cancel=211}
    temporary_values=@{font_size_pt=20;theme='light';theme_index=2;auto_save='false'}
    restored_values=@{font_size_pt='';theme='inherit';theme_index=0;auto_save='false'}
    original_workspace_sha256=$e12InitialHash; temporary_workspace_sha256=$e12TemporaryHashAfterApply
    restored_workspace_sha256=$e12WorkspaceRestoredHash
    common_settings_before_sha256=$e12CommonBeforeHash
    common_settings_after_sha256=$e12CommonAfterHash
    cancel_after_apply_preserved_saved_patch=$true
    initial_caret_point=$e12InitialCaret; temporary_caret_point=$e12TempCaretB
    restored_caret_point=$e12RestoreCaretB
    field_readbacks=[ordered]@{
        common_scope_initial=$e12CommonScope; workspace_scope_session1=$e12ScopeReadbackA
        auto_save_false_before_stage=$e12AutoSaveBeforeA; theme_inherit_before_stage=$e12ThemeBeforeA
        font_size_initial=$e12FontBeforeA; font_size_after_stage=$e12FontStagedTextA
        theme_light_staged=$e12ThemeStagedA; auto_save_false_staged=$e12AutoSaveStagedA
        auto_save_false_after_apply=$e12AutoSaveAfterApplyA; theme_light_after_apply=$e12ThemeAfterApplyA
        font_size_after_apply=$e12FontAfterApplyA
        common_scope_session2=$e12CommonScopeB; workspace_scope_session2=$e12ScopeReadbackB
        auto_save_false_before_restore=$e12AutoSaveBeforeB; theme_light_before_restore=$e12ThemeBeforeB
        font_size_before_restore=$e12FontBeforeB; theme_inherit_staged=$e12ThemeInheritB
        font_size_inherit_staged=$e12FontInheritedB; auto_save_false_inherit_staged=$e12AutoSaveInheritedB
        auto_save_false_after_restore=$e12AutoSaveAfterRestore; theme_inherit_after_restore=$e12ThemeAfterRestore
        font_size_after_restore=$e12FontAfterRestore
    }
    edit205_child_focus=[ordered]@{status='PASS';session1=$e12Edit205FocusEvidenceA;session2=$e12Edit205FocusEvidenceB}
    control_evidence=$e12ControlEvidence; captures=$e12Captures
    rendered_font_family='UNOBSERVED'; rendered_font_identity='UNOBSERVED'
    visual_review='ROOT_REQUIRED'
    not_run=@('Root review of temporary and restored D2D screenshots before any E12 pass',
        'remaining E12 resize/DPI/minimize/restore/IME candidate geometry',
        'U01 44-control color/owner-draw command acceptance')
    reason='Machine settings and document-state oracles complete only after actual execution; E12 remains NOT_RUN until Root reviews real captures.'
}
# Private E14 proposal: bounded save/close recovery on the already-owned Core
# fixture, after E12 restores its settings and before E13 starts.
$script:CurrentEditorRecoveryCaseId='E14'
$e14WaitOwnedDiagnostic={
    param([scriptblock]$Condition,[string]$Description,[int]$WaitMs=3000)
    if ($WaitMs -le 0 -or $WaitMs -gt 3000) { throw 'SETUP_FAILURE: E14 diagnostic wait exceeds its fixed 3s bound.' }
    $deadline=[DateTime]::UtcNow.AddMilliseconds($WaitMs)
    $lastState=$null
    $validSnapshotCount=0
    $lastObserverResponse='No fresh valid diagnostic snapshot has been returned.'
    do {
        $foreground=[MdliteRustAcceptanceNative]::GetForegroundWindow()
        [uint32]$foregroundPid=0
        if ($foreground -ne [IntPtr]::Zero) {
            [void][MdliteRustAcceptanceNative]::GetWindowThreadProcessId($foreground,[ref]$foregroundPid)
        }
        [uint32]$mainPid=0
        $mainThread=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($script:WindowHandle,[ref]$mainPid)
        $openDialogs=@(Get-UiThreadWindows | Where-Object { $_.Class -ceq '#32770' })
        if ($foreground -ne $script:WindowHandle -or $foregroundPid -ne $script:ProcessId -or
            $mainPid -ne $script:ProcessId -or $mainThread -eq 0 -or $openDialogs.Count -ne 0) {
            throw 'ENVIRONMENT_BLOCKED: E14 will not send a diagnostic while Main is unfocused or an owned modal is open.'
        }
        $remaining=[int][Math]::Floor(($deadline-[DateTime]::UtcNow).TotalMilliseconds)
        if ($remaining -le 0) { break }
        $messageTimeout=[int][Math]::Min([int]$DiagnosticMessageTimeoutMs,$remaining)
        try {
            $state=Read-Diagnostic $messageTimeout
            if ($null -eq $state) { throw [System.TimeoutException]::new('No fresh valid diagnostic snapshot was returned.') }
        } catch [System.TimeoutException] {
            $_.Exception.Data['EditorRecoveryBodyDiagnosticFailureKind']='OBSERVER_FAILURE'
            $_.Exception.Data['EditorRecoveryBodyDiagnosticStage']=('E14 bounded wait: '+$Description)
            $_.Exception.Data['LastDiagnosticState']=$lastState
            $_.Exception.Data['ValidSnapshotCount']=$validSnapshotCount
            throw
        }
        $lastState=$state
        $validSnapshotCount++
        $lastObserverResponse='fresh valid snapshot revision={0}, selection={1}..{2}, dirty={3}; condition false' -f
            $state.revision,$state.selection.anchor,$state.selection.head,$state.dirty
        if (& $Condition $state) { return $state }
        $sleepMs=[int][Math]::Min(50,[Math]::Max(1,$remaining))
        Start-Sleep -Milliseconds $sleepMs
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($validSnapshotCount -gt 0) {
        $lastStateSummary=('document_id={0}; revision={1}; source_sha256={2}; selection={3}..{4}; dirty={5}; undo_count={6}; redo_count={7}; tabs={8}; ime_active={9}' -f
            $lastState.document_id,$lastState.revision,(Get-UiTextSha256 ([string]$lastState.source)),
            $lastState.selection.anchor,$lastState.selection.head,$lastState.dirty,
            $lastState.undo_count,$lastState.redo_count,$lastState.tabs,$lastState.ime.active)
        throw [System.InvalidOperationException]::new(('PRODUCT_ASSERTION: E14 bounded diagnostic wait timed out after {0} ms waiting for {1}; {2}; last valid state: {3}.' -f
            $WaitMs,$Description,$lastObserverResponse,$lastStateSummary))
    }
    $exception=[System.TimeoutException]::new(('OBSERVER_FAILURE: E14 bounded diagnostic wait timed out after {0} ms waiting for {1}; no fresh valid snapshot; last observer response: {2}.' -f
        $WaitMs,$Description,$lastObserverResponse))
    $exception.Data['EditorRecoveryBodyDiagnosticFailureKind']='OBSERVER_FAILURE'
    $exception.Data['EditorRecoveryBodyDiagnosticStage']=('E14 bounded wait: '+$Description)
    $exception.Data['LastDiagnosticState']=$null
    $exception.Data['ValidSnapshotCount']=0
    throw $exception
}
$e14CoreSource=[string]$script:EditorRecoveryExpectedSource
$e14CoreBefore=& $e14WaitOwnedDiagnostic { param($value) $true } 'E14 initial owned Core snapshot' 3000
if ([int]$e14CoreBefore.tabs -ne 2 -or $e14CoreBefore.source -cne $e14CoreSource -or
    [bool]$e14CoreBefore.dirty -or [bool]$e14CoreBefore.ime.active -or
    -not [string]::IsNullOrEmpty([string]$e14CoreBefore.ime.preedit)) {
    throw 'SETUP_FAILURE: E14 requires the exact clean Core source in the existing two-tab process.'
}
$e14CoreDocumentId=[long]$e14CoreBefore.document_id
Set-OwnedForeground -Window $script:WindowHandle
Send-Key $script:WindowHandle 0x09 ([uint16[]]@(0x11))
$e14WorkspaceCheck=& $e14WaitOwnedDiagnostic { param($value) [int]$value.tabs -eq 2 -and [long]$value.document_id -ne $e14CoreDocumentId } 'E14 verify the other existing tab' 3000
if ([int]$e14WorkspaceCheck.tabs -ne 2 -or
    [long]$e14WorkspaceCheck.document_id -eq $e14CoreDocumentId -or
    $e14WorkspaceCheck.source -cne $script:EditorRecoveryWorkspaceSource -or
    [bool]$e14WorkspaceCheck.dirty) {
    throw 'ENVIRONMENT_BLOCKED: E14 other tab is not the clean existing Workspace fixture.'
}
Set-OwnedForeground -Window $script:WindowHandle
Send-Key $script:WindowHandle 0x09 ([uint16[]]@(0x11))
$e14CoreBefore=& $e14WaitOwnedDiagnostic { param($value) [long]$value.document_id -eq $e14CoreDocumentId -and $value.source -ceq $e14CoreSource } 'E14 return to the same Core document' 3000
if ([long]$e14CoreBefore.document_id -ne $e14CoreDocumentId -or
    $e14CoreBefore.source -cne $e14CoreSource -or [bool]$e14CoreBefore.dirty) {
    throw 'ENVIRONMENT_BLOCKED: E14 Core document identity changed while checking the other clean tab.'
}
$e14ProfileBefore=Get-EditorRecoveryProfileIdentity $e14CoreBefore.active_input_profile
if ($null -eq $e14ProfileBefore -or
    -not (Test-EditorRecoveryProfileIdentity $script:Report.active_input_profile_initial $e14CoreBefore.active_input_profile)) {
    throw 'ENVIRONMENT_BLOCKED: E14 cannot prove the initial active input-profile identity.'
}
if (-not [bool]$e14CoreBefore.ime.open_status.available -or
    [bool]$e14CoreBefore.ime.open_status.open -or [bool]$e14CoreBefore.ime.active -or
    -not [string]::IsNullOrEmpty([string]$e14CoreBefore.ime.preedit)) {
    throw 'ENVIRONMENT_BLOCKED: E14 requires freshly observed closed app-local IME state; this slice does not close IME.'
}

# Pin the only file this proposal may lock: the harness-created Core fixture.
# The diagnostic schema exposes source and document identity, but not an open
# document path; E10 saved this expected Core source to EditorRecoveryBlankPath.
$e14RunRoot=[IO.Path]::GetFullPath($script:UiRunDirectory).TrimEnd([char[]]@('\','/'))
$e14ExpectedFixtureRoot=[IO.Path]::GetFullPath((Join-Path $e14RunRoot 'workspace'))
$e14FixtureRoot=[IO.Path]::GetFullPath($script:UiFixtureRoot)
$e14CorePath=[IO.Path]::GetFullPath($script:EditorRecoveryBlankPath)
$e14ExpectedCorePath=[IO.Path]::GetFullPath((Join-Path $e14ExpectedFixtureRoot 'new-document.md'))
if (-not [string]::Equals($e14FixtureRoot,$e14ExpectedFixtureRoot,[StringComparison]::OrdinalIgnoreCase) -or
    -not [string]::Equals($e14CorePath,$e14ExpectedCorePath,[StringComparison]::OrdinalIgnoreCase) -or
    -not $e14CorePath.StartsWith(($e14RunRoot+[IO.Path]::DirectorySeparatorChar),[StringComparison]::OrdinalIgnoreCase) -or
    [string]::Equals($e14CorePath,[IO.Path]::GetFullPath($script:EditorRecoveryWorkspacePath),[StringComparison]::OrdinalIgnoreCase) -or
    [string]::Equals($e14CorePath,[IO.Path]::GetFullPath($script:EditorRecoveryPointerPath),[StringComparison]::OrdinalIgnoreCase)) {
    throw 'ENVIRONMENT_BLOCKED: E14 lock target is not the exact synthetic new-document.md under UiRunDirectory.'
}
$e14PathCursor=$e14CorePath
while ($true) {
    $e14PathItem=Get-Item -LiteralPath $e14PathCursor -Force
    if (([IO.FileAttributes]$e14PathItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'ENVIRONMENT_BLOCKED: E14 synthetic file path contains a reparse point.'
    }
    if ([string]::Equals($e14PathCursor,$e14RunRoot,[StringComparison]::OrdinalIgnoreCase)) { break }
    $e14PathParent=[IO.Path]::GetDirectoryName($e14PathCursor)
    if ([string]::IsNullOrEmpty($e14PathParent) -or
        (-not [string]::Equals($e14PathParent,$e14RunRoot,[StringComparison]::OrdinalIgnoreCase) -and
         -not $e14PathParent.StartsWith(($e14RunRoot+[IO.Path]::DirectorySeparatorChar),[StringComparison]::OrdinalIgnoreCase))) {
        throw 'ENVIRONMENT_BLOCKED: E14 path walk escaped UiRunDirectory.'
    }
    $e14PathCursor=$e14PathParent
}
$e14CoreFile=Get-Item -LiteralPath $e14CorePath -Force
if ($e14CoreFile.IsReadOnly) { throw 'SETUP_FAILURE: E14 synthetic Core fixture is unexpectedly read-only.' }
$e14Utf8Strict=[Text.UTF8Encoding]::new($false,$true)
$e14OriginalBytes=[IO.File]::ReadAllBytes($e14CorePath)
$e14OriginalHash=Get-EditorRecoveryBytesSha256 $e14OriginalBytes
$e14OriginalDiskSource=$e14Utf8Strict.GetString($e14OriginalBytes)
if ($e14OriginalDiskSource -cne $e14CoreSource -or
    ($e14OriginalBytes.Length -ge 3 -and $e14OriginalBytes[0] -eq 0xEF -and
        $e14OriginalBytes[1] -eq 0xBB -and $e14OriginalBytes[2] -eq 0xBF)) {
    throw 'SETUP_FAILURE: E14 synthetic Core file bytes do not exactly match the clean saved Core source.'
}
# Ctrl+End establishes the explicit EOF checkpoint without changing source or
# history. The later Undo loop is bounded by the observed new undo entries.
$e14SourceEnd=[Text.Encoding]::UTF8.GetByteCount($e14CoreSource)
Set-OwnedForeground -Window $script:WindowHandle
Send-Key $script:WindowHandle 0x23 ([uint16[]]@(0x11))
$e14Checkpoint=& $e14WaitOwnedDiagnostic {
    param($value)
    [long]$value.document_id -eq $e14CoreDocumentId -and $value.source -ceq $e14CoreSource -and
    [int]$value.selection.anchor -eq $e14SourceEnd -and [int]$value.selection.head -eq $e14SourceEnd -and
    -not [bool]$value.dirty -and [int]$value.undo_count -eq [int]$e14CoreBefore.undo_count -and
    [int]$value.redo_count -eq [int]$e14CoreBefore.redo_count
} 'E14 exact clean Core EOF checkpoint' 3000
if ([long]$e14Checkpoint.document_id -ne $e14CoreDocumentId -or
    $e14Checkpoint.source -cne $e14CoreSource -or
    [int]$e14Checkpoint.selection.anchor -ne $e14SourceEnd -or
    [int]$e14Checkpoint.selection.head -ne $e14SourceEnd -or
    [bool]$e14Checkpoint.dirty -or
    [int]$e14Checkpoint.undo_count -ne [int]$e14CoreBefore.undo_count -or
    [int]$e14Checkpoint.redo_count -ne [int]$e14CoreBefore.redo_count -or
    [long]$e14Checkpoint.revision -ne [long]$e14CoreBefore.revision -or
    -not (Test-EditorRecoveryProfileIdentity $e14ProfileBefore $e14Checkpoint.active_input_profile) -or
    [bool]$e14Checkpoint.ime.open_status.open -or [bool]$e14Checkpoint.ime.active -or
    -not [string]::IsNullOrEmpty([string]$e14Checkpoint.ime.preedit)) {
    throw 'SETUP_FAILURE: E14 EOF checkpoint changed source, identity, saved state, history, profile, or IME state.'
}
$e14CheckpointSelectionJson=ConvertTo-Json -InputObject $e14Checkpoint.selection -Compress -Depth 4
$e14CheckpointAffinity=[string]$e14Checkpoint.selection_affinity
$e14CheckpointUndoCount=[int]$e14Checkpoint.undo_count
$e14CheckpointRedoCount=[int]$e14Checkpoint.redo_count

# Branch 1: ordinary ASCII makes the saved Core doc dirty; normal Alt+F4 asks
# the owned MDLite confirmation; only its safe Cancel button (ID 2) is clicked.
$e14CloseSuffix=' e14close'
$e14CloseSource=$e14CoreSource+$e14CloseSuffix
$e14CloseEnd=[Text.Encoding]::UTF8.GetByteCount($e14CloseSource)
Send-Ascii $e14CloseSuffix $script:WindowHandle
$e14CloseDraft=& $e14WaitOwnedDiagnostic {
    param($value)
    $value.source -ceq $e14CloseSource -and [long]$value.document_id -eq $e14CoreDocumentId -and
    [int]$value.selection.anchor -eq $e14CloseEnd -and [int]$value.selection.head -eq $e14CloseEnd -and
    [bool]$value.dirty -and [int]$value.undo_count -gt $e14CheckpointUndoCount -and [int]$value.redo_count -eq 0
} 'E14 dirty Core suffix after normal ASCII input' 3000
if ([long]$e14CloseDraft.document_id -ne $e14CoreDocumentId -or
    [int]$e14CloseDraft.undo_count -le $e14CheckpointUndoCount -or
    [int]$e14CloseDraft.redo_count -ne 0) {
    throw 'PRODUCT_ASSERTION: E14 close suffix did not remain on the same dirty Core document with new undo history.'
}
$e14CloseUndoBudget=[int]$e14CloseDraft.undo_count-$e14CheckpointUndoCount
if ($e14CloseUndoBudget -gt 16) { throw 'SETUP_FAILURE: E14 close suffix produced an unexpectedly large Undo budget.' }
$e14CloseDraftSelectionJson=ConvertTo-Json -InputObject $e14CloseDraft.selection -Compress -Depth 4
$e14CloseDraftAffinity=[string]$e14CloseDraft.selection_affinity
$e14CloseDraftProfile=Get-EditorRecoveryProfileIdentity $e14CloseDraft.active_input_profile
$e14ClosePrompt=[IntPtr]::Zero
Send-Key $script:WindowHandle 0x73 ([uint16[]]@(0x12))
$e14ClosePrompt=Wait-OwnedConfirmation 3000
if ($e14ClosePrompt -eq [IntPtr]::Zero) {
    throw 'OBSERVER_FAILURE: E14 normal Alt+F4 did not expose the owned MDLite close confirmation; no diagnostic was sent while pending.'
}
Assert-UiOwnedWindow $e14ClosePrompt
if ([MdliteRustAcceptanceNative]::WindowClass($e14ClosePrompt) -cne '#32770' -or
    [MdliteRustAcceptanceNative]::WindowTitle($e14ClosePrompt) -cne 'MDLite') {
    throw 'ENVIRONMENT_BLOCKED: E14 close modal owner/class/title differs from the current Rust confirmation.'
}
$e14CloseDialogOwner=[MdliteRustAcceptanceNative]::GetParent($e14ClosePrompt)
[uint32]$e14CloseDialogPid=0
$e14CloseDialogThread=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e14ClosePrompt,[ref]$e14CloseDialogPid)
[uint32]$e14CloseMainPid=0
$e14CloseMainThread=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($script:WindowHandle,[ref]$e14CloseMainPid)
if ($e14CloseDialogOwner -ne $script:WindowHandle -or
    $e14CloseDialogPid -ne $script:ProcessId -or $e14CloseMainPid -ne $script:ProcessId -or
    $e14CloseDialogThread -eq 0 -or $e14CloseDialogThread -ne $e14CloseMainThread) {
    throw 'ENVIRONMENT_BLOCKED: E14 close modal is not owned by the exact Main HWND/process/thread.'
}
$e14CloseCancel=Get-UiControlById -Root $e14ClosePrompt -ControlId 2 -Class 'BUTTON' -RequireVisible -RequireEnabled
if ($e14CloseCancel.Id -ne 2 -or
    -not [string]::Equals([string]$e14CloseCancel.Class,'BUTTON',[StringComparison]::OrdinalIgnoreCase) -or
    -not $e14CloseCancel.Visible -or -not $e14CloseCancel.Enabled) {
    throw 'ENVIRONMENT_BLOCKED: E14 owned close Cancel control ID 2 is not uniquely safe to activate.'
}
[uint32]$e14CancelPid=0
$e14CancelThread=[MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e14CloseCancel.Handle,[ref]$e14CancelPid)
if ([MdliteRustAcceptanceNative]::GetParent($e14CloseCancel.Handle) -ne $e14ClosePrompt -or
    $e14CancelPid -ne $script:ProcessId -or $e14CancelThread -ne $e14CloseDialogThread) {
    throw 'ENVIRONMENT_BLOCKED: E14 Cancel button is not a child on the exact owned dialog thread.'
}
Invoke-UiDialogButton $e14ClosePrompt 2
Wait-UiWindowGone $e14ClosePrompt 3000
if ([MdliteRustAcceptanceNative]::GetForegroundWindow() -ne $script:WindowHandle) {
    throw 'ENVIRONMENT_BLOCKED: close Cancel did not return foreground to Main; do not query through a pending modal.'
}
$e14CloseAfterCancel=& $e14WaitOwnedDiagnostic {
    param($value)
    $value.source -ceq $e14CloseSource -and [long]$value.document_id -eq [long]$e14CloseDraft.document_id -and
    [long]$value.revision -eq [long]$e14CloseDraft.revision -and
    [int]$value.selection.anchor -eq $e14CloseEnd -and [int]$value.selection.head -eq $e14CloseEnd -and
    [string]$value.selection_affinity -ceq $e14CloseDraftAffinity -and [bool]$value.dirty -and
    [int]$value.undo_count -eq [int]$e14CloseDraft.undo_count -and [int]$value.redo_count -eq [int]$e14CloseDraft.redo_count
} 'E14 source/history after close Cancel and modal removal' 3000
if ([long]$e14CloseAfterCancel.document_id -ne $e14CloseDraft.document_id -or
    [long]$e14CloseAfterCancel.revision -ne [long]$e14CloseDraft.revision -or
    (ConvertTo-Json -InputObject $e14CloseAfterCancel.selection -Compress -Depth 4) -cne $e14CloseDraftSelectionJson -or
    [string]$e14CloseAfterCancel.selection_affinity -cne $e14CloseDraftAffinity -or
    [bool]$e14CloseAfterCancel.dirty -ne [bool]$e14CloseDraft.dirty -or
    [int]$e14CloseAfterCancel.undo_count -ne [int]$e14CloseDraft.undo_count -or
    [int]$e14CloseAfterCancel.redo_count -ne [int]$e14CloseDraft.redo_count -or
    -not (Test-EditorRecoveryProfileIdentity $e14CloseDraftProfile $e14CloseAfterCancel.active_input_profile)) {
    throw 'PRODUCT_ASSERTION: E14 Cancel did not preserve the exact dirty source, document, selection, affinity, history, and profile.'
}
if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e14CorePath))) -cne $e14OriginalHash) {
    throw 'ENVIRONMENT_BLOCKED: E14 close Cancel changed the synthetic saved Core file; external/recovery branches remain out of scope.'
}
$e14CloseUndo=$e14CloseAfterCancel
for ($i=0; $i -lt $e14CloseUndoBudget; $i++) {
    $e14BeforeUndo=$e14CloseUndo
    $e14UndoCountExpected=[int]$e14BeforeUndo.undo_count-1
    $e14RedoCountExpected=[int]$e14BeforeUndo.redo_count+1
    Set-OwnedForeground -Window $script:WindowHandle
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
    $e14CloseUndo=& $e14WaitOwnedDiagnostic {
        param($value)
        $sourceText=[string]$value.source
        [long]$value.document_id -eq $e14CoreDocumentId -and
        [int]$value.undo_count -eq $e14UndoCountExpected -and [int]$value.redo_count -eq $e14RedoCountExpected -and
        $sourceText.StartsWith($e14CoreSource,[StringComparison]::Ordinal) -and
        $sourceText.Length -lt ([string]$e14BeforeUndo.source).Length
    } ('E14 close-branch Undo {0} completion' -f ($i+1)) 3000
}
if ($e14CloseUndo.source -cne $e14CoreSource -or
    [int]$e14CloseUndo.selection.anchor -ne $e14SourceEnd -or
    [int]$e14CloseUndo.selection.head -ne $e14SourceEnd -or
    [string]$e14CloseUndo.selection_affinity -cne $e14CheckpointAffinity -or
    [bool]$e14CloseUndo.dirty -or
    [int]$e14CloseUndo.undo_count -ne $e14CheckpointUndoCount -or
    [int]$e14CloseUndo.redo_count -ne $e14CloseUndoBudget -or
    -not (Test-EditorRecoveryProfileIdentity $e14ProfileBefore $e14CloseUndo.active_input_profile) -or
    (Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e14CorePath))) -cne $e14OriginalHash) {
    throw 'PRODUCT_ASSERTION: bounded E14 Undo did not restore the saved Core checkpoint after close Cancel.'
}

# Branch 2: reacquire the clean checkpoint, lock only its exact synthetic file,
# cause the normal Ctrl+S read to fail, then release and retry through the UI.
$e14SaveCheckpoint=$e14CloseUndo
if ([long]$e14SaveCheckpoint.document_id -ne $e14CoreDocumentId -or
    $e14SaveCheckpoint.source -cne $e14CoreSource -or
    (ConvertTo-Json -InputObject $e14SaveCheckpoint.selection -Compress -Depth 4) -cne $e14CheckpointSelectionJson -or
    [string]$e14SaveCheckpoint.selection_affinity -cne $e14CheckpointAffinity -or
    [bool]$e14SaveCheckpoint.dirty -or
    (Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e14CorePath))) -cne $e14OriginalHash) {
    throw 'SETUP_FAILURE: E14 save-lock branch did not begin from the exact clean saved Core checkpoint.'
}
$e14SaveSuffix=' e14save'
$e14SaveSource=$e14CoreSource+$e14SaveSuffix
$e14SaveEnd=[Text.Encoding]::UTF8.GetByteCount($e14SaveSource)
Send-Ascii $e14SaveSuffix $script:WindowHandle
$e14SaveDraft=& $e14WaitOwnedDiagnostic {
    param($value)
    $value.source -ceq $e14SaveSource -and [long]$value.document_id -eq $e14CoreDocumentId -and
    [int]$value.selection.anchor -eq $e14SaveEnd -and [int]$value.selection.head -eq $e14SaveEnd -and
    [bool]$value.dirty -and [int]$value.undo_count -gt [int]$e14SaveCheckpoint.undo_count -and [int]$value.redo_count -eq 0
} 'E14 dirty Core suffix before locked Ctrl+S' 3000
if ([long]$e14SaveDraft.document_id -ne $e14CoreDocumentId -or
    [int]$e14SaveDraft.undo_count -le [int]$e14SaveCheckpoint.undo_count -or
    [int]$e14SaveDraft.redo_count -ne 0) {
    throw 'PRODUCT_ASSERTION: E14 save suffix did not remain on the same dirty Core document with new undo history.'
}
$e14SaveUndoBudget=[int]$e14SaveDraft.undo_count-[int]$e14SaveCheckpoint.undo_count
if ($e14SaveUndoBudget -gt 16) { throw 'SETUP_FAILURE: E14 save suffix produced an unexpectedly large Undo budget.' }
$e14SaveDraftSelectionJson=ConvertTo-Json -InputObject $e14SaveDraft.selection -Compress -Depth 4
$e14SaveDraftAffinity=[string]$e14SaveDraft.selection_affinity
$e14SaveDraftProfile=Get-EditorRecoveryProfileIdentity $e14SaveDraft.active_input_profile
$e14ExpectedSaveBytes=$e14Utf8Strict.GetBytes($e14SaveSource)
$e14SaveLock=$null
$e14SavePrimaryFailure=$null
try {
    if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e14CorePath))) -cne $e14OriginalHash) {
        throw 'ENVIRONMENT_BLOCKED: E14 synthetic Core bytes changed before exclusive lock acquisition.'
    }
    $e14SaveLock=[IO.File]::Open($e14CorePath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::None)
    $e14SaveLock.Position=0
    $e14LockedBytes=[byte[]]::new([int]$e14SaveLock.Length)
    $e14LockRead=0
    while ($e14LockRead -lt $e14LockedBytes.Length) {
        $e14ReadNow=$e14SaveLock.Read($e14LockedBytes,$e14LockRead,$e14LockedBytes.Length-$e14LockRead)
        if ($e14ReadNow -le 0) { throw 'OBSERVER_FAILURE: E14 exclusive lock stream ended before the baseline bytes were read.' }
        $e14LockRead+=$e14ReadNow
    }
    if ((Get-EditorRecoveryBytesSha256 $e14LockedBytes) -cne $e14OriginalHash) {
        throw 'ENVIRONMENT_BLOCKED: E14 exclusive lock does not protect the expected baseline bytes.'
    }
    Set-OwnedForeground -Window $script:WindowHandle
    Send-Key $script:WindowHandle 0x53 ([uint16[]]@(0x11))
    $e14SaveForeground=[MdliteRustAcceptanceNative]::GetForegroundWindow()
    [uint32]$e14SaveForegroundPid=0
    if ($e14SaveForeground -ne [IntPtr]::Zero) {
        [void][MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e14SaveForeground,[ref]$e14SaveForegroundPid)
    }
    if ($e14SaveForeground -ne $script:WindowHandle -or $e14SaveForegroundPid -ne $script:ProcessId) {
        throw 'ENVIRONMENT_BLOCKED: E14 unexpected modal after locked Save; leave it pending without diagnostic/OK/Escape.'
    }
    $e14SaveFailure=& $e14WaitOwnedDiagnostic {
        param($value)
        ([string]$value.status).StartsWith('保存失敗 — 本文は保持しています:',[StringComparison]::Ordinal) -and
        [long]$value.document_id -eq [long]$e14SaveDraft.document_id -and
        [long]$value.revision -eq [long]$e14SaveDraft.revision -and
        $value.source -ceq $e14SaveSource -and
        [int]$value.selection.anchor -eq [int]$e14SaveDraft.selection.anchor -and
        [int]$value.selection.head -eq [int]$e14SaveDraft.selection.head -and
        [string]$value.selection_affinity -ceq $e14SaveDraftAffinity -and
        [bool]$value.dirty -and
        [int]$value.undo_count -eq [int]$e14SaveDraft.undo_count -and
        [int]$value.redo_count -eq [int]$e14SaveDraft.redo_count
    } 'E14 source-preserving Save failure status while exclusive lock remains held' 3000
    if ([long]$e14SaveFailure.document_id -ne $e14SaveDraft.document_id -or
        [long]$e14SaveFailure.revision -ne [long]$e14SaveDraft.revision -or
        $e14SaveFailure.source -cne $e14SaveSource -or
        (ConvertTo-Json -InputObject $e14SaveFailure.selection -Compress -Depth 4) -cne $e14SaveDraftSelectionJson -or
        [string]$e14SaveFailure.selection_affinity -cne $e14SaveDraftAffinity -or
        -not [bool]$e14SaveFailure.dirty -or
        [int]$e14SaveFailure.undo_count -ne [int]$e14SaveDraft.undo_count -or
        [int]$e14SaveFailure.redo_count -ne [int]$e14SaveDraft.redo_count -or
        -not (Test-EditorRecoveryProfileIdentity $e14SaveDraftProfile $e14SaveFailure.active_input_profile)) {
        throw 'PRODUCT_ASSERTION: E14 failed Save changed the exact draft source, selection, dirty state, history, or profile.'
    }
} catch {
    $e14SavePrimaryFailure=$_.Exception
    throw
} finally {
    if ($null -ne $e14SaveLock) {
        try { $e14SaveLock.Dispose(); $e14SaveLock=$null }
        catch {
            if ($null -eq $e14SavePrimaryFailure) { throw }
            $script:Report.e14_lock_release_error=$_.Exception.Message
        }
    }
}
if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e14CorePath))) -cne $e14OriginalHash) {
    throw 'ENVIRONMENT_BLOCKED: E14 file bytes changed during the failed locked Save; external-change branch is outside this slice.'
}
Set-OwnedForeground -Window $script:WindowHandle
Send-Key $script:WindowHandle 0x53 ([uint16[]]@(0x11))
$e14RetryForeground=[MdliteRustAcceptanceNative]::GetForegroundWindow()
if ($e14RetryForeground -ne $script:WindowHandle) {
    throw 'ENVIRONMENT_BLOCKED: E14 normal Save retry opened an unexpected modal; leave it pending.'
}
$e14SaveRetry=& $e14WaitOwnedDiagnostic {
    param($value)
    $value.source -ceq $e14SaveSource -and
    [long]$value.document_id -eq $e14CoreDocumentId -and
    [int]$value.selection.anchor -eq $e14SaveEnd -and
    [int]$value.selection.head -eq $e14SaveEnd -and
    -not [bool]$value.dirty -and
    [int]$value.undo_count -eq [int]$e14SaveDraft.undo_count -and
    [int]$value.redo_count -eq [int]$e14SaveDraft.redo_count -and
    [long]$value.revision -eq [long]$e14SaveDraft.revision -and
    [string]$value.selection_affinity -ceq $e14SaveDraftAffinity -and
    (Test-EditorRecoveryProfileIdentity $e14ProfileBefore $value.active_input_profile)
} 'E14 normal Ctrl+S retry after releasing the exact synthetic file lock' 3000
if ([int]$e14SaveRetry.undo_count -ne [int]$e14SaveDraft.undo_count -or
    [int]$e14SaveRetry.redo_count -ne [int]$e14SaveDraft.redo_count -or
    [long]$e14SaveRetry.revision -ne [long]$e14SaveDraft.revision -or
    [string]$e14SaveRetry.selection_affinity -cne $e14SaveDraftAffinity -or
    -not (Test-EditorRecoveryProfileIdentity $e14ProfileBefore $e14SaveRetry.active_input_profile) -or
    (Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e14CorePath))) -cne
        (Get-EditorRecoveryBytesSha256 $e14ExpectedSaveBytes)) {
    throw 'PRODUCT_ASSERTION: E14 released-lock retry did not save exact suffix bytes while preserving document history/profile.'
}

$e14SaveUndo=$e14SaveRetry
for ($i=0; $i -lt $e14SaveUndoBudget; $i++) {
    $e14BeforeSaveUndo=$e14SaveUndo
    $e14SaveUndoCountExpected=[int]$e14BeforeSaveUndo.undo_count-1
    $e14SaveRedoCountExpected=[int]$e14BeforeSaveUndo.redo_count+1
    Set-OwnedForeground -Window $script:WindowHandle
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
    $e14SaveUndo=& $e14WaitOwnedDiagnostic {
        param($value)
        $sourceText=[string]$value.source
        [long]$value.document_id -eq $e14CoreDocumentId -and
        [int]$value.undo_count -eq $e14SaveUndoCountExpected -and [int]$value.redo_count -eq $e14SaveRedoCountExpected -and
        [bool]$value.dirty -and $sourceText.StartsWith($e14CoreSource,[StringComparison]::Ordinal) -and
        $sourceText.Length -lt ([string]$e14BeforeSaveUndo.source).Length
    } ('E14 saved-suffix Undo {0} completion' -f ($i+1)) 3000
}
if ($e14SaveUndo.source -cne $e14CoreSource -or
    [int]$e14SaveUndo.selection.anchor -ne $e14SourceEnd -or
    [int]$e14SaveUndo.selection.head -ne $e14SourceEnd -or
    [string]$e14SaveUndo.selection_affinity -cne $e14CheckpointAffinity -or
    -not [bool]$e14SaveUndo.dirty -or
    [int]$e14SaveUndo.undo_count -ne $e14CheckpointUndoCount -or
    [int]$e14SaveUndo.redo_count -ne $e14SaveUndoBudget) {
    throw 'PRODUCT_ASSERTION: E14 Undo after successful suffix Save did not restore the original source checkpoint.'
}
Set-OwnedForeground -Window $script:WindowHandle
Send-Key $script:WindowHandle 0x53 ([uint16[]]@(0x11))
if ([MdliteRustAcceptanceNative]::GetForegroundWindow() -ne $script:WindowHandle) {
    throw 'ENVIRONMENT_BLOCKED: E14 restore Save opened an unexpected modal; leave it pending.'
}
$e14Restore=& $e14WaitOwnedDiagnostic {
    param($value)
    $value.source -ceq $e14CoreSource -and
    [long]$value.document_id -eq $e14CoreDocumentId -and
    [int]$value.selection.anchor -eq $e14SourceEnd -and
    [int]$value.selection.head -eq $e14SourceEnd -and
    -not [bool]$value.dirty -and
    [int]$value.undo_count -eq $e14CheckpointUndoCount -and
    [int]$value.redo_count -eq $e14SaveUndoBudget -and
    [string]$value.selection_affinity -ceq $e14CheckpointAffinity -and
    (Test-EditorRecoveryProfileIdentity $e14ProfileBefore $value.active_input_profile)
} 'E14 ordinary Ctrl+S restore of the original Core source before E13' 3000
$e14ExpectedOriginalBytes=$e14Utf8Strict.GetBytes($e14CoreSource)
if ([int]$e14Restore.undo_count -ne $e14CheckpointUndoCount -or
    [int]$e14Restore.redo_count -ne $e14SaveUndoBudget -or
    [string]$e14Restore.selection_affinity -cne $e14CheckpointAffinity -or
    -not (Test-EditorRecoveryProfileIdentity $e14ProfileBefore $e14Restore.active_input_profile) -or
    $script:EditorRecoveryExpectedSource -cne $e14CoreSource -or
    (Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e14CorePath))) -cne
        (Get-EditorRecoveryBytesSha256 $e14ExpectedOriginalBytes)) {
    throw 'PRODUCT_ASSERTION: E14 ordinary Save did not restore exact original Core bytes/source/history/profile before E13.'
}
Set-OwnedForeground -Window $script:WindowHandle
$script:CurrentEditorRecoveryCaseId='E14'
Set-EditorRecoveryCase 'E14' 'NOT_RUN' @{
    close_cancel=@{dialog_title='MDLite';cancel_button_id=2;source_disk_unchanged=$true;undo_budget=$e14CloseUndoBudget}
    locked_save=@{failure_status_prefix='保存失敗 — 本文は保持しています:';failure_is_main_status_not_modal=$true;lock_disposed_before_retry=$true;retry_source_sha256=(Get-UiTextSha256 $e14SaveSource);restored_original_sha256=(Get-EditorRecoveryBytesSha256 $e14ExpectedOriginalBytes);undo_budget=$e14SaveUndoBudget}
    recovery_artifacts='NOT_ENUMERATED_OR_MODIFIED; application persistence is separate'
    core_document_id=$e14CoreDocumentId;original_source_sha256=Get-UiTextSha256 $e14CoreSource
    original_file_sha256=$e14OriginalHash;final_file_sha256=(Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($e14CorePath)))
    machine_substeps='PASS'
    not_run=@('external file-change branch','IME close/restore branch','recovery/deferred-cleanup branch','temporary/restored screenshot review')
    reason='Both bounded safe-UI branches ran and their machine oracles passed; whole E14 remains NOT_RUN pending external-file-change, IME close/restore, recovery/deferred-cleanup, and visual-review branches.'
}
        $script:CurrentEditorRecoveryCaseId='E13'
        $savedState=Read-Diagnostic
        $saved=[string]$savedState.source
        $savedDocumentId=[long]$savedState.document_id
        Save-ActiveFixture $saved $script:EditorRecoveryBlankPath
        $expectedNoBom=[Text.UTF8Encoding]::new($false).GetBytes($saved)
        if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($script:EditorRecoveryBlankPath))) -cne (Get-EditorRecoveryBytesSha256 $expectedNoBom)) {
            throw 'PRODUCT_ASSERTION: E13 blank fixture save bytes differ from exact expected UTF-8 bytes.'
        }
        [void](Switch-ToSource $script:EditorRecoveryWorkspaceSource)
        $structured=[string](Read-Diagnostic).source; $end=[Text.Encoding]::UTF8.GetByteCount($structured)
        Send-Key $script:InputWindowHandle 0x23 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoveryDocument $structured $end $false 'E13 BOM/CRLF EOF')
        Send-Ascii ' tail' $script:InputWindowHandle
        $structuredExpected=$structured+' tail'
        [void](Assert-EditorRecoveryDocument $structuredExpected ([Text.Encoding]::UTF8.GetByteCount($structuredExpected)) $true 'E13 BOM/CRLF edit')
        Send-Key $script:InputWindowHandle 0x53 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoveryDocument $structuredExpected ([Text.Encoding]::UTF8.GetByteCount($structuredExpected)) $false 'E13 BOM/CRLF save')
        $preamble=[Text.UTF8Encoding]::new($true).GetPreamble(); $body=[Text.UTF8Encoding]::new($false).GetBytes($structuredExpected)
        $expectedBytes=[byte[]]::new($preamble.Length+$body.Length)
        [Array]::Copy($preamble,0,$expectedBytes,0,$preamble.Length); [Array]::Copy($body,0,$expectedBytes,$preamble.Length,$body.Length)
        if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($script:EditorRecoveryWorkspacePath))) -cne (Get-EditorRecoveryBytesSha256 $expectedBytes)) {
            throw 'PRODUCT_ASSERTION: E13 BOM/CRLF save bytes differ from exact expected bytes.'
        }
        $expectedWorkspaceSettingsBefore='schema_version = 1'+$script:EditorRecoveryLf+'auto_save = false'+$script:EditorRecoveryLf
        $workspaceSettingsBeforeBytes=[Text.UTF8Encoding]::new($false).GetBytes($expectedWorkspaceSettingsBefore)
        if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($script:EditorRecoveryWorkspaceSettingsPath))) -cne
            (Get-EditorRecoveryBytesSha256 $workspaceSettingsBeforeBytes)) {
            throw 'SETUP_FAILURE: E13 Workspace settings are not the exact auto_save=false fixture established before the prior dirty-state oracles.'
        }
        $workspaceSettingsBeforeHash=(Get-FileHash -LiteralPath $script:EditorRecoveryWorkspaceSettingsPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $commonSettingsPath=Join-Path $script:UiLocalAppData 'MDLite/settings.toml'
        $commonSettingsBeforeHash=Get-UiSafeFileHash $commonSettingsPath
        Invoke-UiControlById $script:WindowHandle 114
        $e13SettingsWindow=Wait-UiThreadWindow 'MDLiteSettings' '' 3000
        Assert-UiOwnedWindow $e13SettingsWindow
        [uint32]$e13SettingsOwnerPid=0
        [void][MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e13SettingsWindow,[ref]$e13SettingsOwnerPid)
        if ($e13SettingsOwnerPid -ne $script:ProcessId) {
            throw 'ENVIRONMENT_BLOCKED: E13 SettingsGUI owner PID differs from the exact EditorRecovery process.'
        }
        $e13Scope=Get-UiControlById -Root $e13SettingsWindow -ControlId 200 -Class 'COMBOBOX' -RequireVisible -RequireEnabled
        Invoke-UiControlClick $e13SettingsWindow $e13Scope
        Send-Key $e13SettingsWindow 0x24
        Send-Key $e13SettingsWindow 0x28
        $e13ScopeReadback=Get-UiOwnedSettingsComboSelection $e13SettingsWindow 200 1 'Workspace'
        $e13ScopeIndex=[int]$e13ScopeReadback.index
        $e13ScopeText=[string]$e13ScopeReadback.text
        $e13AutoSave=Get-UiControlById -Root $e13SettingsWindow -ControlId 201 -Class 'COMBOBOX' -RequireVisible -RequireEnabled
        Invoke-UiControlClick $e13SettingsWindow $e13AutoSave
        Send-Key $e13SettingsWindow 0x24
        Send-Key $e13SettingsWindow 0x28
        $e13AutoSaveReadback=Get-UiOwnedSettingsComboSelection $e13SettingsWindow 201 1 'true'
        $e13AutoSaveIndex=[int]$e13AutoSaveReadback.index
        $e13AutoSaveText=[string]$e13AutoSaveReadback.text
        Set-OwnedForeground -Window $e13SettingsWindow
        $e13SettingsForeground=[MdliteRustAcceptanceNative]::GetForegroundWindow()
        [uint32]$e13SettingsForegroundPid=0
        if ($e13SettingsForeground -ne [IntPtr]::Zero) {
            [void][MdliteRustAcceptanceNative]::GetWindowThreadProcessId($e13SettingsForeground,[ref]$e13SettingsForegroundPid)
        }
        if ($e13SettingsForeground -ne $e13SettingsWindow -or $e13SettingsForegroundPid -ne $script:ProcessId) {
            throw 'ENVIRONMENT_BLOCKED: E13 Workspace and auto_save combo readbacks were not focused in the exact owned SettingsGUI PID.'
        }
        Invoke-UiDialogButton $e13SettingsWindow 210
        $e13SettingsNotice=Wait-UiThreadWindow '#32770' '設定' 3000
        Assert-UiOwnedWindow $e13SettingsNotice
        Invoke-UiDialogButton $e13SettingsNotice 1
        Wait-UiWindowGone $e13SettingsNotice
        $expectedWorkspaceSettingsAfter=$expectedWorkspaceSettingsBefore.Replace('auto_save = false','auto_save = true')
        $expectedWorkspaceSettingsAfterBytes=[Text.UTF8Encoding]::new($false).GetBytes($expectedWorkspaceSettingsAfter)
        $workspaceSettingsAfterBytes=[IO.File]::ReadAllBytes($script:EditorRecoveryWorkspaceSettingsPath)
        $commonSettingsAfterHash=Get-UiSafeFileHash $commonSettingsPath
        if ((Get-EditorRecoveryBytesSha256 $workspaceSettingsAfterBytes) -cne
            (Get-EditorRecoveryBytesSha256 $expectedWorkspaceSettingsAfterBytes)) {
            throw 'PRODUCT_ASSERTION: E13 Workspace settings patch did not set auto_save=true while preserving the exact existing keys and file text.'
        }
        if ($commonSettingsAfterHash -cne $commonSettingsBeforeHash) {
            throw 'PRODUCT_ASSERTION: E13 SettingsGUI changed the isolated common settings file while applying the Workspace patch.'
        }
        Invoke-UiDialogButton $e13SettingsWindow 211
        Wait-UiWindowGone $e13SettingsWindow
        $workspaceSettingsAfterCloseBytes=[IO.File]::ReadAllBytes($script:EditorRecoveryWorkspaceSettingsPath)
        $commonSettingsAfterCloseHash=Get-UiSafeFileHash $commonSettingsPath
        if ((Get-EditorRecoveryBytesSha256 $workspaceSettingsAfterCloseBytes) -cne
            (Get-EditorRecoveryBytesSha256 $expectedWorkspaceSettingsAfterBytes)) {
            throw 'PRODUCT_ASSERTION: E13 Workspace auto_save=true patch changed after SettingsGUI closed.'
        }
        if ($commonSettingsAfterCloseHash -cne $commonSettingsBeforeHash) {
            throw 'PRODUCT_ASSERTION: E13 isolated common settings changed when SettingsGUI closed the Workspace form.'
        }
        $workspaceSettingsAfterHash=Get-EditorRecoveryBytesSha256 $workspaceSettingsAfterCloseBytes
        $script:Report.workspace_settings['e13_settings_apply']=[ordered]@{
            setup_configured_auto_save=[bool]$script:Report.workspace_settings.configured_auto_save
            scope_combo_index=$e13ScopeIndex; scope_combo_text=$e13ScopeText
            auto_save_combo_index=$e13AutoSaveIndex; auto_save_combo_text=$e13AutoSaveText
            scope_combo_hwnd=$e13ScopeReadback.hwnd; auto_save_combo_hwnd=$e13AutoSaveReadback.hwnd
            combo_owner_pids=@([int]$e13ScopeReadback.owner_pid,[int]$e13AutoSaveReadback.owner_pid)
            settings_hwnd='0x{0:X}' -f $e13SettingsWindow.ToInt64()
            settings_owner_pid=[int]$e13SettingsOwnerPid
            focused_hwnd='0x{0:X}' -f $e13SettingsForeground.ToInt64()
            focused_pid=[int]$e13SettingsForegroundPid
            application_readback='PASS'; workspace_scope_selected=$true; auto_save_true_selected=$true
            workspace_settings_path=$script:EditorRecoveryWorkspaceSettingsPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
            workspace_settings_before_sha256=$workspaceSettingsBeforeHash
            workspace_settings_after_sha256=$workspaceSettingsAfterHash
            exact_existing_keys_and_bytes_preserved=((Get-EditorRecoveryBytesSha256 $workspaceSettingsAfterCloseBytes) -ceq (Get-EditorRecoveryBytesSha256 $expectedWorkspaceSettingsAfterBytes))
            isolated_common_settings_path=$commonSettingsPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
            isolated_common_settings_before_sha256=$commonSettingsBeforeHash
            isolated_common_settings_after_apply_sha256=$commonSettingsAfterHash
            isolated_common_settings_after_close_sha256=$commonSettingsAfterCloseHash
            isolated_common_settings_unchanged=($commonSettingsAfterCloseHash -ceq $commonSettingsBeforeHash)
        }
        $autoSaveStart=Switch-ToSource $saved
        if ([long]$autoSaveStart.document_id -ne $savedDocumentId -or [int]$autoSaveStart.tabs -ne 2 -or
            $autoSaveStart.source -cne $saved -or [bool]$autoSaveStart.dirty) {
            throw 'PRODUCT_ASSERTION: E13 auto-save trial did not return to the original clean LF synthetic document in the same two-tab Workspace session.'
        }
        if ($saved.Contains([char]13) -or -not $saved.Contains([char]10)) {
            throw 'SETUP_FAILURE: E13 auto-save trial requires the existing synthetic target to contain LF and no CR.'
        }
        $autoSaveDocumentPath=$script:EditorRecoveryBlankPath
        $autoSaveBaselineBytes=[Text.UTF8Encoding]::new($false).GetBytes($saved)
        if (-not (Test-Path -LiteralPath $autoSaveDocumentPath -PathType Leaf) -or
            (Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($autoSaveDocumentPath))) -cne (Get-EditorRecoveryBytesSha256 $autoSaveBaselineBytes)) {
            throw 'SETUP_FAILURE: E13 auto-save target is not the exact existing LF/no-BOM saved checkpoint.'
        }
        $autoSaveDocumentEnd=[Text.Encoding]::UTF8.GetByteCount($saved)
        $autoSaveUndoBefore=[int]$autoSaveStart.undo_count
        $autoSaveRedoBefore=[int]$autoSaveStart.redo_count
        Send-Key $script:InputWindowHandle 0x23 ([uint16[]]@(0x11))
        $autoSaveBaseline=Assert-EditorRecoverySelection $saved $savedDocumentId `
            $autoSaveDocumentEnd $autoSaveDocumentEnd $false $autoSaveUndoBefore $autoSaveRedoBefore 'E13 LF autosave checkpoint at EOF'
        $autoSaveSuffix=' autosave witness'
        $autoSaveExpectedSource=$saved+$autoSaveSuffix
        $autoSaveExpectedEnd=[Text.Encoding]::UTF8.GetByteCount($autoSaveExpectedSource)
        $autoSaveExpectedBytes=[Text.UTF8Encoding]::new($false).GetBytes($autoSaveExpectedSource)
        $autoSaveExpectedHash=Get-EditorRecoveryBytesSha256 $autoSaveExpectedBytes
        Send-Ascii $autoSaveSuffix $script:InputWindowHandle
        $autoSaved=Wait-Diagnostic {
            param($state)
            if ([long]$state.document_id -ne $savedDocumentId -or $state.source -cne $autoSaveExpectedSource -or
                [int]$state.selection.anchor -ne $autoSaveExpectedEnd -or [int]$state.selection.head -ne $autoSaveExpectedEnd -or
                [bool]$state.dirty -or [int]$state.undo_count -ne ($autoSaveUndoBefore+1) -or
                [int]$state.redo_count -ne 0 -or [long]$state.revision -le [long]$autoSaveBaseline.revision) { return $false }
            try {
                return (Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($autoSaveDocumentPath))) -ceq $autoSaveExpectedHash
            } catch [IO.IOException] {
                return $false
            }
        } 'E13 auto-save writes exact LF/no-BOM source and clears dirty state' $TimeoutMs
        [void](Save-EditorRecoveryEvidence 'E13' 'autosaved-lf-suffix')
        Send-Key $script:InputWindowHandle 0x5A ([uint16[]]@(0x11))
        $autoSaveRestored=Wait-Diagnostic {
            param($state)
            if ([long]$state.document_id -ne $savedDocumentId -or $state.source -cne $saved -or
                [int]$state.selection.anchor -ne $autoSaveDocumentEnd -or [int]$state.selection.head -ne $autoSaveDocumentEnd -or
                [bool]$state.dirty -or [int]$state.undo_count -ne $autoSaveUndoBefore -or
                [int]$state.redo_count -ne 1 -or [long]$state.revision -le [long]$autoSaved.revision) { return $false }
            try {
                return (Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($autoSaveDocumentPath))) -ceq
                    (Get-EditorRecoveryBytesSha256 $autoSaveBaselineBytes)
            } catch [IO.IOException] {
                return $false
            }
        } 'E13 Undo auto-saves the exact original LF checkpoint' $TimeoutMs
        [void](Save-EditorRecoveryEvidence 'E13' 'autosave-undo-restored-checkpoint')
        $script:Report.workspace_settings['e13_autosave_trial']=[ordered]@{
            status='PASS'; document_path=$autoSaveDocumentPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
            document_id=$savedDocumentId; workspace_session_tabs=[int]$autoSaveRestored.tabs
            source_eol='LF'; bom='NONE'; suffix=$autoSaveSuffix
            source_before_sha256=Get-UiTextSha256 $saved
            source_after_autosave_sha256=Get-UiTextSha256 ([string]$autoSaved.source)
            source_after_undo_sha256=Get-UiTextSha256 ([string]$autoSaveRestored.source)
            revision_before=[long]$autoSaveBaseline.revision; revision_after_autosave=[long]$autoSaved.revision
            revision_after_undo=[long]$autoSaveRestored.revision
            undo_before=$autoSaveUndoBefore; undo_after_autosave=[int]$autoSaved.undo_count; undo_after_undo=[int]$autoSaveRestored.undo_count
            redo_before=$autoSaveRedoBefore; redo_after_autosave=[int]$autoSaved.redo_count; redo_after_undo=[int]$autoSaveRestored.redo_count
            dirty_after_autosave=[bool]$autoSaved.dirty; dirty_after_undo=[bool]$autoSaveRestored.dirty
            disk_sha256_before=(Get-EditorRecoveryBytesSha256 $autoSaveBaselineBytes)
            disk_sha256_after_autosave=$autoSaveExpectedHash
            disk_sha256_after_undo=(Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($autoSaveDocumentPath)))
            transient_dirty_required=$false; manual_save_during_wait=$false
        }
        $blankAfterAutosave=Switch-ToSource $saved
        if ($blankAfterAutosave.source -cne $saved -or [bool]$blankAfterAutosave.dirty -or
            (Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($script:EditorRecoveryBlankPath))) -cne
            (Get-EditorRecoveryBytesSha256 $autoSaveBaselineBytes)) {
            throw 'PRODUCT_ASSERTION: E13 auto-save Undo did not preserve the exact clean original blank-fixture bytes before normal close.'
        }
        [void](Switch-ToSource $structuredExpected)
        [void](Assert-EditorRecoveryDocument $structuredExpected ([Text.Encoding]::UTF8.GetByteCount($structuredExpected)) $false 'E13 BOM/CRLF checkpoint unchanged after autosave trial')
        if ((Get-EditorRecoveryBytesSha256 ([IO.File]::ReadAllBytes($script:EditorRecoveryWorkspacePath))) -cne (Get-EditorRecoveryBytesSha256 $expectedBytes)) {
            throw 'PRODUCT_ASSERTION: E13 autosave trial changed the explicitly saved BOM/CRLF Workspace bytes.'
        }
        [void](Switch-ToSource $saved)
        [void](Save-EditorRecoveryEvidence 'E13' 'pre-close-source')
        Close-EditorRecoveryProcess 'continuous-workspace-session'
        $script:UiFixtureRoot=Join-Path $script:UiRunDirectory 'workspace'
        Start-UiParityProcess @($script:UiFixtureRoot,$script:EditorRecoveryWorkspacePath,$script:EditorRecoveryBlankPath) $script:DiagnosticPath
        [void](Wait-Diagnostic {param($s)[int]$s.tabs -eq 2} 'E13 reopen fixtures')
        $script:Report.workspace_settings['workspace_root_readbacks']['reopen'] =
            Get-EditorRecoveryWorkspaceRootReceipt $script:UiFixtureRoot
        [void](Switch-ToSource $saved)
        [void](Assert-EditorRecoveryDocument $saved ([Text.Encoding]::UTF8.GetByteCount($saved)) $false 'E13 reopened exact blank source')
        [void](Save-EditorRecoveryEvidence 'E13' 'reopened-core-source')
        [void](Switch-ToSource $structuredExpected)
        [void](Assert-EditorRecoveryDocument $structuredExpected ([Text.Encoding]::UTF8.GetByteCount($structuredExpected)) $false 'E13 reopened exact BOM/CRLF source')
        [void](Save-EditorRecoveryEvidence 'E13' 'reopened-source')
        Close-EditorRecoveryProcess 'reopen-verification'
        # Machine substeps are recorded above; reopened captures need visual review before E13 can pass.
        Set-EditorRecoveryCase 'E13' 'NOT_RUN' @{
            save_bytes='PASS'; reopen='PASS'; normal_close='PASS'
            settings_gui_apply=$script:Report.workspace_settings.e13_settings_apply
            autosave_trial=$script:Report.workspace_settings.e13_autosave_trial
            autosave='PASS'; undo_autosave_original_checkpoint='PASS'
            root_actual_gui='PASS'; reopened_display_capture='PASS_CAPTURED'
            physical_input='NOT_RUN'; human_subjective_acceptance='NOT_RUN_SEPARATE'
            reason='Machine save, LF autosave, Undo checkpoint, normal close/reopen, and source checks passed; E13 remains NOT_RUN pending visual review of the reopened Core and BOM/CRLF Workspace captures. Physical input and subjective Human acceptance remain separate.'
        }

        $script:Report.operations += [pscustomobject]@{ name='continuous EditorRecovery core sequence';cases=@('E01','E02','E03','E04','E05','E06','E07','E08','E09','E10','E11','E12','E14','E13') }
    } catch {
        $failureRecord = $_
        $failureMessage = $failureRecord.Exception.Message
        $script:Report.failure=$failureMessage
        $script:Report.failure_exception_type = $failureRecord.Exception.GetType().FullName
        $script:Report.failure_exception_message = $failureMessage
        $script:Report.failure_script_stack_trace = [string]$failureRecord.ScriptStackTrace
        $lastDiagnosticState=$failureRecord.Exception.Data['LastDiagnosticState']
        if ($null -ne $lastDiagnosticState) {
            try {
                $validSnapshotCount=0
                if ($failureRecord.Exception.Data.Contains('ValidSnapshotCount')) {
                    $validSnapshotCount=[int]$failureRecord.Exception.Data['ValidSnapshotCount']
                }
                if ($validSnapshotCount -lt 1) { throw 'Primary timeout snapshot has no valid read count.' }
                $description=[string]$failureRecord.Exception.Data['EditorRecoveryBodyDiagnosticStage']
                if ([string]::IsNullOrWhiteSpace($description)) { $description=[string]$failureRecord.Exception.Data['Description'] }
                if ([string]::IsNullOrWhiteSpace($description)) { $description=$failureMessage }
                $capturePath=Join-Path $script:UiRunDirectory 'primary-timeout-last-diagnostic.json'
                $payload=[ordered]@{description=$description;valid_snapshot_count=$validSnapshotCount;snapshot=$lastDiagnosticState}
                $captureBytes=[Text.UTF8Encoding]::new($false).GetBytes(($payload|ConvertTo-Json -Depth 40))
                $captureStream=[IO.File]::Open($capturePath,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
                try { $captureStream.Write($captureBytes,0,$captureBytes.Length) } finally { $captureStream.Dispose() }
                $captureHash=(Get-FileHash -LiteralPath $capturePath -Algorithm SHA256).Hash.ToLowerInvariant()
                $script:Report.primary_timeout_last_diagnostic=[ordered]@{
                    relative_path=[IO.Path]::GetFileName($capturePath);sha256=$captureHash
                    valid_snapshot_count=$validSnapshotCount;description=$description
                }
            } catch { $script:Report.primary_timeout_last_diagnostic_capture_error=$_.Exception.Message }
        }
        $bodyDiagnosticFailureKind = [string]$failureRecord.Exception.Data['EditorRecoveryBodyDiagnosticFailureKind']
        if ($failureRecord.Exception.Data.Contains('EditorRecoveryBodyDiagnosticStage')) {
            $script:Report.failure_body_diagnostic_stage = [string]$failureRecord.Exception.Data['EditorRecoveryBodyDiagnosticStage']
        }
        $script:Report.failure_kind = if ($bodyDiagnosticFailureKind -ceq 'OBSERVER_FAILURE') {
            'OBSERVER_FAILURE'
        } elseif ($failureMessage -match '^(OBSERVER_FAILURE|SEMANTIC_WAIT_TIMEOUT|SETUP_FAILURE|ENVIRONMENT_BLOCKED):') {
            $Matches[1]
        } elseif ($failureMessage -match '^PRODUCT_ASSERTION:') {
            'PRODUCT_ASSERTION'
        } elseif ($failureMessage -match 'foreground|focus|SendInput|OWNERSHIP_GATE') {
            'ENVIRONMENT_BLOCKED'
        } else {
            'HARNESS_FAILURE'
        }
        $id=[string]$script:CurrentEditorRecoveryCaseId
        if ($id) {
            $status=if ($script:Report.failure_kind -in @('OBSERVER_FAILURE','SEMANTIC_WAIT_TIMEOUT','SETUP_FAILURE','ENVIRONMENT_BLOCKED')) {'BLOCKED'} else {'FAIL'}
            Set-EditorRecoveryCase $id $status @{failure_kind=$script:Report.failure_kind;reason=$failureMessage}
        }
        $script:Report.cleanup_note='Only harness-started processes may be closed; baseline processes were not touched.'
    } finally {
        $failureBeforeCleanup = $script:Report.failure
        $failureKindBeforeCleanup = $script:Report.failure_kind
        $previousImeRestoration = $script:Report.ime_profile_restoration
        $finalImeRestoration = Restore-EditorRecoveryInitialImeStateForFailure
        if ($previousImeRestoration) {
            $finalImeRestoration.previous_attempt = $previousImeRestoration
        }
        $script:Report.ime_profile_restoration = $finalImeRestoration
        $integrityFailures = @()
        $script:Report.executable_sha256_after=(Get-FileHash -LiteralPath $ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
        $script:Report.source_tree_sha256_after=Get-SourceTreeSha256
        $script:Report.harness_sha256_after=(Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($script:Report.executable_sha256_after -cne $script:Report.executable_sha256_before) {$integrityFailures += 'Release executable changed during run.'}
        if ($script:Report.source_tree_sha256_after -cne $script:Report.source_tree_sha256_before) {$integrityFailures += 'Rust source/build manifests changed during run.'}
        if ($script:Report.harness_sha256_after -cne $script:Report.harness_sha256) {$integrityFailures += 'Harness changed during run.'}
        if ($failureBeforeCleanup) {
            $script:Report.failure = $failureBeforeCleanup
            $script:Report.failure_kind = $failureKindBeforeCleanup
            $script:Report.secondary_failures = @($script:Report.secondary_failures) + $integrityFailures
        } elseif ($integrityFailures.Count -gt 0) {
            $script:Report.failure = $integrityFailures[0]
            $script:Report.failure_kind = 'INTEGRITY_FAILURE'
            if ($integrityFailures.Count -gt 1) {
                $script:Report.secondary_failures = @($integrityFailures | Select-Object -Skip 1)
            }
        }
        $statuses=@($script:Report.editor_recovery_cases | ForEach-Object {[string]$_.status})
        if ($script:Report.failure_kind -in @('PRODUCT_ASSERTION','HARNESS_FAILURE','INTEGRITY_FAILURE') -or
            @($script:Report.secondary_failures).Count -gt 0 -or $statuses -contains 'FAIL') {$script:Report.status='FAIL'}
        elseif ($script:Report.failure_kind -in @('OBSERVER_FAILURE','SEMANTIC_WAIT_TIMEOUT','SETUP_FAILURE','ENVIRONMENT_BLOCKED')) {$script:Report.status='BLOCKED'}
        elseif ($statuses -contains 'BLOCKED') {$script:Report.status='BLOCKED'}
        elseif ($null -eq $script:Report.ime_profile_restoration -or $script:Report.ime_profile_restoration.status -cne 'PASS') {$script:Report.status='BLOCKED'}
        elseif ($statuses -contains 'NOT_RUN') {$script:Report.status='NOT_RUN'}
        else {$script:Report.status='PASS'}
        if ($script:Process) {
            try {$script:Process.Refresh();$script:Report.process_still_running=-not $script:Process.HasExited}
            catch {$script:Report.process_still_running=$null}
            $script:Process.Dispose()
        }
        $script:Report.ended_utc=[DateTime]::UtcNow.ToString('o')
        $script:Report.result_path=$script:ReportPath
        [IO.File]::WriteAllText($script:ReportPath,($script:Report | ConvertTo-Json -Depth 16),[Text.UTF8Encoding]::new($false))
    }
    if ($script:Report.status -ne 'PASS') { throw ('EditorRecovery {0}: {1}. Evidence: {2}' -f $script:Report.status,$script:Report.failure,$script:ReportPath) }
    Write-Output ('PASS: EditorRecovery. Evidence: {0}' -f $script:ReportPath)
}


if ($EditorRecovery) {
    Invoke-EditorRecoverySuite
    return
}

if ($UiParity -or $StartupSmoke) {
    Invoke-UiParitySuite
    if ($script:Report.status -ne 'PASS') {
        throw ('{0} {1}. Evidence: .codex-runtime/editor-acceptance/{2}/result.json' -f
            $script:Report.suite, $script:Report.status, $script:Report.run_id)
    }
    return
}

try {
    if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
        throw "Release executable is missing: $ExecutablePath"
    }
    if ($ExecutablePath -notmatch '[\\/]target[\\/]release[\\/]mdlite\.exe$') {
        throw 'Only target/release/mdlite.exe is accepted by this harness.'
    }

    $script:Report.git_head = (& git -C $ProjectRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not pin the current Git HEAD.' }
    $script:Report.executable_sha256_before = (Get-FileHash -LiteralPath $ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:Report.source_tree_sha256_before = Get-SourceTreeSha256

    $runId = '{0}-{1}' -f [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ'), [guid]::NewGuid().ToString('N')
    $runDirectory = Join-Path $RunRoot $runId
    [void][IO.Directory]::CreateDirectory($runDirectory)
    $script:ReportPath = Join-Path $runDirectory 'result.json'
    $script:DiagnosticPath = Join-Path $runDirectory 'diagnostic.json'
    $script:ForegroundLogPath = Join-Path $runDirectory 'foreground.log'
    $script:Report.foreground_log_path = $script:ForegroundLogPath
    $blankPath = Join-Path $runDirectory 'synthetic-empty.md'
    $structuredPath = Join-Path $runDirectory 'synthetic-editor.md'

    $newline = [string][char]10
    $fence = ([string][char]96) * 3
    $structuredSource = '| left | right |' + $newline +
        '| --- | --- |' + $newline +
        '| cell | value |' + $newline + $newline +
        $fence + 'rust' + $newline +
        'let source = "Markdown";' + $newline +
        '## asdkl' + $newline +
        $fence + $newline + $newline

    [IO.File]::WriteAllText($blankPath, '', [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($structuredPath, $structuredSource, [Text.UTF8Encoding]::new($false))
    $script:Report.fixtures = @(
        @{ path = $blankPath; initial_sha256 = (Get-FileHash -LiteralPath $blankPath -Algorithm SHA256).Hash.ToLowerInvariant() },
        @{ path = $structuredPath; initial_sha256 = (Get-FileHash -LiteralPath $structuredPath -Algorithm SHA256).Hash.ToLowerInvariant() }
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $ExecutablePath
    $startInfo.WorkingDirectory = $ProjectRoot
    $startInfo.UseShellExecute = $false
    $startInfo.Arguments = (ConvertTo-WindowsArgument $blankPath) + ' ' +
        (ConvertTo-WindowsArgument $structuredPath) + ' --diagnostic-path ' +
        (ConvertTo-WindowsArgument $script:DiagnosticPath)
    $script:Process = [Diagnostics.Process]::Start($startInfo)
    if ($null -eq $script:Process) { throw 'Could not start the pinned Release executable.' }
    $script:ProcessId = [uint32]$script:Process.Id
    $script:Report.pid = [int]$script:ProcessId

    $windowDeadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $script:Process.Refresh()
        if ($script:Process.HasExited) { throw 'The owned Release process exited before creating its main window.' }
        $script:WindowHandle = $script:Process.MainWindowHandle
        if ($script:WindowHandle -ne [IntPtr]::Zero -and
            [MdliteRustAcceptanceNative]::WindowClass($script:WindowHandle) -eq 'MDLiteRustEditor') {
            break
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $windowDeadline)
    if ($script:WindowHandle -eq [IntPtr]::Zero -or
        [MdliteRustAcceptanceNative]::WindowClass($script:WindowHandle) -ne 'MDLiteRustEditor') {
        throw 'The launched process did not expose its expected native editor HWND before timeout.'
    }
    Assert-OwnedWindow $script:WindowHandle
    $script:Report.hwnd = '0x{0:X}' -f $script:WindowHandle.ToInt64()
    $script:Report.visible = [MdliteRustAcceptanceNative]::IsWindowVisible($script:WindowHandle)
    $script:Report.dpi = [int][MdliteRustAcceptanceNative]::GetDpiForWindow($script:WindowHandle)
    if (-not $script:Report.visible -or $script:Report.dpi -le 0) {
        throw 'The exact owned native window was not visible or did not report a valid DPI.'
    }
    Set-OwnedForeground -WaitMs 60000

    $initial = Wait-Diagnostic {
        param($state)
        [int]$state.tabs -eq 2 -and
        ($state.source -ceq '' -or $state.source -ceq $structuredSource)
    } 'startup with both owned synthetic fixtures'
    $script:Report.tabs = [int]$initial.tabs
    $script:Report.active_input_profile = $initial.active_input_profile
    $script:Report.ime_open_status = $initial.ime.open_status
    Add-Observation 'visible Release startup' $initial
    if ([bool]$initial.raw) { throw 'The synthetic fixtures did not open in Live mode.' }
    if ([bool]$initial.ime.active -or -not [string]::IsNullOrEmpty([string]$initial.ime.preedit)) {
        throw 'The owned editor began with active composition; no keyboard input was sent.'
    }

    $state = Switch-ToSource $structuredSource
    if ([int]$state.tables -ne 1) { throw 'The structured fixture did not expose its single Markdown table.' }
    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x24 ([uint16[]]@(0x11))
    $tableStart = $structuredSource.IndexOf('left', [StringComparison]::Ordinal)
    $tableEnd = $tableStart + 4
    $state = Wait-Diagnostic { param($value) $true } 'fresh table-caret diagnostic'
    for ($step = 0; $step -lt 8 -and
        -not ([int]$state.selection.anchor -eq [int]$state.selection.head -and
              [int]$state.selection.head -ge $tableStart -and
              [int]$state.selection.head -le $tableEnd); $step++) {
        $oldHead = [int]$state.selection.head
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x27
        $state = Wait-Diagnostic {
            param($value)
            $value.source -ceq $structuredSource -and [int]$value.selection.head -ne $oldHead
        } 'caret movement into the first table cell'
    }
    if (-not ([int]$state.selection.anchor -eq [int]$state.selection.head -and
              [int]$state.selection.head -ge $tableStart -and
              [int]$state.selection.head -le $tableEnd)) {
        throw 'Keyboard navigation did not reach the first table cell within the bounded steps.'
    }
    $rightStart = $structuredSource.IndexOf('right', [StringComparison]::Ordinal)
    $rightEnd = $rightStart + 'right'.Length
    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x09
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $structuredSource -and
        [int]$value.selection.anchor -eq $rightStart -and
        [int]$value.selection.head -eq $rightEnd
    } 'Tab navigation to the next table cell'
    if ([int]$state.tables -ne 1) { throw 'Tab navigation changed the table count.' }
    Add-Observation 'Tab selects the whole destination cell' $state

    $editedTableSource = $structuredSource.Remove($rightStart, $rightEnd - $rightStart).Insert($rightStart, 'X')
    Send-Ascii 'X'
    $editedEnd = $rightStart + 1
    $state = Wait-Document $editedTableSource $editedEnd $editedEnd $true
    if ([int]$state.tables -ne 1) { throw 'Typing in a table cell changed the table count.' }
    Add-Observation 'replace selected table cell through normal input' $state
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
    $state = Wait-Document $structuredSource $rightStart $rightEnd $false
    if ([int]$state.tables -ne 1) { throw 'Undo did not restore the original table structure.' }
    Add-Observation 'table cell Undo restores source and selected range' $state
    Send-Key $script:WindowHandle 0x59 ([uint16[]]@(0x11))
    $state = Wait-Document $editedTableSource $editedEnd $editedEnd $true
    Add-Observation 'table cell Redo restores edit' $state
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
    $state = Wait-Document $structuredSource $rightStart $rightEnd $false

    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x23 ([uint16[]]@(0x11))
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($structuredSource)
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $structuredSource -and
        [int]$value.selection.anchor -eq $sourceEnd -and
        [int]$value.selection.head -eq $sourceEnd
    } 'Ctrl+End at the final blank paragraph'
    Add-Observation 'Ctrl+End reaches the EOF blank paragraph' $state

    Send-Key $script:WindowHandle 0x0D
    $expectedStructured = $structuredSource + $newline
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedStructured)
    $state = Wait-Document $expectedStructured $sourceEnd $sourceEnd $true
    Add-Observation 'Enter appends a single EOF line break' $state
    Send-Key $script:WindowHandle 0x0D
    $expectedStructured += $newline
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedStructured)
    $state = Wait-Document $expectedStructured $sourceEnd $sourceEnd $true
    Add-Observation 'repeated Enter preserves the next blank line' $state
    Send-Ascii '## asdkl'
    $expectedStructured += '## asdkl'
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedStructured)
    $state = Wait-Document $expectedStructured $sourceEnd $sourceEnd $true
    if ([bool]$state.raw) { throw 'Markdown heading input did not remain in Live mode.' }
    if ($state.source.IndexOf('## asdkl', [StringComparison]::Ordinal) -lt 0 -or
        -not $state.source.EndsWith('## asdkl', [StringComparison]::Ordinal)) {
        throw 'The same Markdown spelling was not preserved inside code and at the document tail.'
    }
    Add-Observation 'write ## heading outside fenced literal at EOF' $state
    Save-ActiveFixture $expectedStructured $structuredPath
    $script:Report.fixtures[1].saved_source_sha256 = (Get-FileHash -LiteralPath $structuredPath -Algorithm SHA256).Hash.ToLowerInvariant()

    $state = Switch-ToSource ''
    if ([bool]$state.dirty -or [int]$state.selection.anchor -ne 0 -or [int]$state.selection.head -ne 0) {
        throw 'The empty synthetic fixture did not start clean at source offset zero.'
    }
    Add-Observation 'empty document starts at source offset zero' $state
    Send-Ascii 'alpha beta'
    $state = Wait-Document 'alpha beta' 10 10 $true
    if ([int]$state.undo_count -le 0) { throw 'Normal text input did not create document history.' }
    Add-Observation 'normal text input in empty document' $state

    Set-OwnedForeground
    $expectedHead = 10
    foreach ($move in 1..2) {
        Send-Key $script:WindowHandle 0x25
        $expectedHead--
        $state = Wait-Document 'alpha beta' $expectedHead $expectedHead $true
        Add-Observation ('repeated Left key ' + $move) $state
    }
    Send-Key $script:WindowHandle 0x27 ([uint16[]]@(0x10))
    $state = Wait-Document 'alpha beta' 8 9 $true
    Add-Observation 'Shift+Right extends selection' $state
    Send-Key $script:WindowHandle 0x27 ([uint16[]]@(0x10))
    $state = Wait-Document 'alpha beta' 8 10 $true
    Add-Observation 'repeated Shift+Right extends selection' $state
    Send-Key $script:WindowHandle 0x25
    $state = Wait-Document 'alpha beta' 8 8 $true
    Add-Observation 'Left collapses the selection to its start' $state
    Send-Key $script:WindowHandle 0x23 ([uint16[]]@(0x11))
    $state = Wait-Document 'alpha beta' 10 10 $true
    $script:Report.discrete_repeat_status = 'PASS: discrete repeated Left and Shift+Right key events ran through the visible window; physical held-key typematic remains NOT_RUN.'

    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x25 ([uint16[]]@(0x11, 0x10))
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq 'alpha beta' -and
        [int]$value.selection.anchor -eq 10 -and
        [int]$value.selection.head -lt 10
    } 'Ctrl+Shift+Left reverse word selection'
    $selected = $state.source.Substring([int]$state.selection.head, [int]($state.selection.anchor - $state.selection.head))
    if ($selected -cne 'beta') { throw "Ctrl+Shift+Left selected '$selected' instead of the expected final word." }
    Add-Observation 'Ctrl+Shift+Left with virtual-key modifiers' $state

    Send-Ascii 'gamma'
    $state = Wait-Document 'alpha gamma' 11 11 $true
    Add-Observation 'replace selected word' $state
    $beforeUndoCount = [int]$state.undo_count
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
    $state = Wait-Document 'alpha beta' 10 6 $true
    if ([int]$state.undo_count -ge $beforeUndoCount -or [int]$state.redo_count -le 0) {
        throw 'Ctrl+Z did not restore source, reverse selection, and redo history.'
    }
    Add-Observation 'Ctrl+Z restores source and reverse selection' $state
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11, 0x10))
    $state = Wait-Document 'alpha gamma' 11 11 $true
    Add-Observation 'Ctrl+Shift+Z restores replacement' $state
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
    $state = Wait-Document 'alpha beta' 10 6 $true
    Send-Key $script:WindowHandle 0x59 ([uint16[]]@(0x11))
    $state = Wait-Document 'alpha gamma' 11 11 $true
    Add-Observation 'Ctrl+Y restores replacement' $state

    Send-UnicodeText 'かな'
    $state = Wait-Document 'alpha gammaかな' 17 17 $true
    Add-Observation 'Japanese Unicode packet text (separate from IME composition)' $state
    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x0D
    $expectedBlank = 'alpha gammaかな' + $newline
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedBlank)
    $state = Wait-Document $expectedBlank $sourceEnd $sourceEnd $true
    Add-Observation 'Enter in empty-document scenario' $state
    Send-Ascii '## heading'
    $expectedBlank += '## heading'
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedBlank)
    $state = Wait-Document $expectedBlank $sourceEnd $sourceEnd $true
    Add-Observation 'write Markdown after Enter in Live mode' $state

    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x45 ([uint16[]]@(0x11))
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $expectedBlank -and [bool]$value.raw
    } 'Ctrl+E Source mode switch'
    Add-Observation 'Ctrl+E enters Source mode' $state
    Send-Ascii '!'
    $expectedBlank += '!'
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedBlank)
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $expectedBlank -and
        [int]$value.selection.anchor -eq $sourceEnd -and
        [int]$value.selection.head -eq $sourceEnd -and
        [bool]$value.dirty -and [bool]$value.raw
    } 'Source mode input'
    Add-Observation 'write in Source mode' $state
    Send-Key $script:WindowHandle 0x45 ([uint16[]]@(0x11))
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $expectedBlank -and -not [bool]$value.raw
    } 'Ctrl+E Live mode switch'
    if ([bool]$state.ime.active -or -not [string]::IsNullOrEmpty([string]$state.ime.preedit)) {
        throw 'The synthetic text path left composition active.'
    }
    Add-Observation 'Ctrl+E returns to Live mode' $state

    $state = Invoke-ConfiguredImeAcceptance $expectedBlank $sourceEnd
    if ([string]$script:Report.ime_status -eq 'PENDING') {
        throw 'The configured IME acceptance path returned without PASS or a concrete BLOCKED reason.'
    }
    $imeOriginalSource = $expectedBlank
    $state = Wait-Diagnostic { param($value) $true } 'fresh diagnostic after IME acceptance'
    $imePassed = ([string]$script:Report.ime_status).StartsWith('PASS:', [StringComparison]::Ordinal)
    if ($imePassed) {
        if (-not $state.source.StartsWith($imeOriginalSource, [StringComparison]::Ordinal) -or
            $state.source.Length -le $imeOriginalSource.Length) {
            throw 'The configured IME PASS result did not leave a committed suffix in the active source.'
        }
    } elseif ($state.source -cne $imeOriginalSource) {
        throw 'The IME BLOCKED path changed the source unexpectedly.'
    }
    $expectedBlank = [string]$state.source
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedBlank)
    if ([int]$state.selection.anchor -ne $sourceEnd -or
        [int]$state.selection.head -ne $sourceEnd -or
        -not [bool]$state.dirty -or [bool]$state.ime.active -or
        -not [string]::IsNullOrEmpty([string]$state.ime.preedit)) {
        throw 'Post-IME source, caret, dirty, or composition state is not ready for the close/save check.'
    }
    Add-Observation 'post-IME source and caret baseline for close/save' $state

    Assert-OwnedWindow $script:WindowHandle
    $closeResult = [IntPtr]::Zero
    [void][MdliteRustAcceptanceNative]::SendMessageTimeoutW(
        $script:WindowHandle, [uint32]$WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero,
        [uint32]$SMTO_ABORTIFHUNG, [uint32]800, [ref]$closeResult)
    $dialog = Wait-OwnedConfirmation
    if ($dialog -eq [IntPtr]::Zero) {
        throw 'Dirty WM_CLOSE did not present the owned MDLite save confirmation.'
    }
    $script:PromptOpen = $true
    $script:Report.unsaved_close_prompt = @{
        hwnd = '0x{0:X}' -f $dialog.ToInt64()
        pid = [int]$script:ProcessId
        class = [MdliteRustAcceptanceNative]::WindowClass($dialog)
        title = [MdliteRustAcceptanceNative]::WindowTitle($dialog)
        response = 'Escape cancels close and preserves the dirty synthetic document.'
    }
    Send-Key $dialog 0x1B
    $script:PromptOpen = $false
    Set-OwnedForeground
    $state = Wait-Document $expectedBlank $sourceEnd $sourceEnd $true
    Add-Observation 'cancel dirty close preserves unsaved source' $state

    Save-ActiveFixture $expectedBlank $blankPath
    $script:Report.fixtures[0].saved_source_sha256 = (Get-FileHash -LiteralPath $blankPath -Algorithm SHA256).Hash.ToLowerInvariant()

    Assert-OwnedWindow $script:WindowHandle
    $closeResult = [IntPtr]::Zero
    $closedMessage = [MdliteRustAcceptanceNative]::SendMessageTimeoutW(
        $script:WindowHandle, [uint32]$WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero,
        [uint32]$SMTO_ABORTIFHUNG, [uint32]1000, [ref]$closeResult)
    if (-not $closedMessage -or -not $script:Process.WaitForExit(2500)) {
        throw 'The clean owned Release window did not close normally before timeout.'
    }
    $script:Report.normal_close = 'PASS after dirty-close cancellation and saving both synthetic fixtures.'

    $script:Report.executable_sha256_after = (Get-FileHash -LiteralPath $ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:Report.source_tree_sha256_after = Get-SourceTreeSha256
    if ($script:Report.executable_sha256_after -cne $script:Report.executable_sha256_before) {
        throw 'The executable changed while this acceptance run was in progress.'
    }
    if ($script:Report.source_tree_sha256_after -cne $script:Report.source_tree_sha256_before) {
        throw 'Rust source or build manifests changed while this acceptance run was in progress.'
    }
    $script:Report.status = 'PASS'
} catch {
    $script:Report.status = 'FAIL'
    $script:Report.failure = $_.Exception.Message
} finally {
    if ($script:PromptOpen -and $script:WindowHandle -ne [IntPtr]::Zero) {
        try {
            $dialog = Wait-OwnedConfirmation 500
            if ($dialog -ne [IntPtr]::Zero) {
                Send-Key $dialog 0x1B
                $script:PromptOpen = $false
            }
        } catch {
            $script:Report.cleanup_note = 'An owned confirmation could not be dismissed safely; the process was left running.'
        }
    }
    if ($null -ne $script:Process) {
        try {
            $script:Process.Refresh()
            $script:Report.process_still_running = -not $script:Process.HasExited
            if ($script:Report.status -ne 'PASS' -and -not $script:Process.HasExited) {
                $script:Report.cleanup_note = 'Failed runs leave the exact owned process open so unsaved synthetic text remains available for inspection.'
            }
        } catch {
            $script:Report.process_still_running = $null
        }
        $script:Process.Dispose()
    } else {
        $script:Report.process_still_running = $false
    }
    if (-not [string]::IsNullOrWhiteSpace($script:ReportPath)) {
        $script:Report.ended_utc = [DateTime]::UtcNow.ToString('o')
        $script:Report.result_path = $script:ReportPath
        $json = $script:Report | ConvertTo-Json -Depth 12
        [IO.File]::WriteAllText($script:ReportPath, $json, [Text.UTF8Encoding]::new($false))
    }
}

if ($script:Report.status -ne 'PASS') {
    throw "Rust GUI acceptance failed: $($script:Report.failure) Evidence: $($script:ReportPath)"
}
Write-Output "PASS: Release native window acceptance. Evidence: $($script:ReportPath)"
