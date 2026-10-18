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
    private const uint INPUT_KEYBOARD = 1;
    private const uint KEYEVENTF_KEYUP = 0x0002;
    private const uint KEYEVENTF_UNICODE = 0x0004;

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
        if (!IsOwnedSaveAsDialog(dialog, main, expectedProcessId) || GetForegroundWindow() != dialog)
            return IntPtr.Zero;
        uint actualProcessId;
        uint threadId = GetWindowThreadProcessId(dialog, out actualProcessId);
        if (threadId == 0 || actualProcessId != expectedProcessId)
            return IntPtr.Zero;
        GUITHREADINFO info = new GUITHREADINFO();
        info.cbSize = (uint)Marshal.SizeOf(info);
        if (!GetGUIThreadInfo(threadId, ref info) || info.hwndActive != dialog ||
            info.hwndFocus == IntPtr.Zero || info.hwndCaret != info.hwndFocus)
            return IntPtr.Zero;
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
            return IntPtr.Zero;
        IntPtr ancestor = combo;
        for (int depth = 0; depth < 16 && ancestor != IntPtr.Zero; depth++)
        {
            uint ancestorProcessId;
            if (!IsOwnedWindow(ancestor, expectedProcessId) ||
                GetWindowThreadProcessId(ancestor, out ancestorProcessId) != threadId ||
                ancestorProcessId != expectedProcessId)
                return IntPtr.Zero;
            if (ancestor == dialog) return edit;
            ancestor = GetParent(ancestor);
        }
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

    public static int DragOwnedBetween(
        IntPtr expectedForeground, uint expectedProcessId,
        int startX, int startY, int endX, int endY)
    {
        if (GetForegroundWindow() != expectedForeground ||
            !IsOwnedWindow(expectedForeground, expectedProcessId))
            return -1;
        RECT rect;
        if (!GetWindowRect(expectedForeground, out rect) ||
            startX < rect.Left || startX >= rect.Right ||
            startY < rect.Top || startY >= rect.Bottom ||
            endX < rect.Left || endX >= rect.Right ||
            endY < rect.Top || endY >= rect.Bottom)
            return -2;
        if (!SetCursorPos(startX, startY))
            return -3;
        INPUT down = new INPUT();
        down.type = 0;
        down.data.mi.dwFlags = 0x0002;
        if (SendInput(1, new INPUT[] { down }, Marshal.SizeOf(typeof(INPUT))) != 1)
            return -4;
        SetCursorPos(endX, endY);
        INPUT move = new INPUT();
        move.type = 0;
        move.data.mi.dwFlags = 0x0001;
        INPUT up = new INPUT();
        up.type = 0;
        up.data.mi.dwFlags = 0x0004;
        INPUT[] finish = new INPUT[] { move, up };
        return (int)SendInput(2, finish, Marshal.SizeOf(typeof(INPUT)));
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

function Cancel-EditorRecoveryImeComposition($Baseline, $CurrentState, [string]$Description, [int]$MaxEscapes = 3) {
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
            Assert-EditorRecoveryImeInvariant $value $Baseline ($Description + ' Escape stage')
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
        $cancelResult = Cancel-EditorRecoveryImeComposition $baseline $state 'cancellation of owned composition' 3
        $state = $cancelResult.state
        if (-not $cancelResult.cancelled) {
            throw 'ENVIRONMENT_BLOCKED: owned IME composition remained active after three normal Escape stages.'
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
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x20
        try {
            $converted = Wait-EditorRecoveryStableState -WaitMs 10000 -Description 'stable Space conversion' -Condition {
                param($value)
                Assert-EditorRecoveryImeInvariant $value $baseline 'Space conversion'
                [bool]$value.ime.active -and
                    -not [string]::IsNullOrEmpty([string]$value.ime.preedit) -and
                    [string]$value.ime.preedit -cne $rawPreedit -and
                    (Test-EditorRecoveryJapaneseText ([string]$value.ime.preedit))
            }
        } catch {
            if ($_.Exception.Message -match '^OBSERVER_FAILURE:|^PRODUCT_ASSERTION:') { throw }
            $script:Report.ime_space_conversion = @{
                status='NOT_RUN'; before_preedit=$rawPreedit
                reason='Space did not expose a distinct stable Japanese conversion in the owner-thread diagnostic.'
            }
            $script:Report.ime_status = "BLOCKED: Space conversion for '$reading' was not observable as a stable, changed Japanese preedit."
            return $_.Exception.Data['LastDiagnosticState']
        }
        $convertedPreedit = [string]$converted.ime.preedit
        $script:Report.ime_space_conversion = @{
            status='PASS'; before_preedit=$rawPreedit; after_preedit=$convertedPreedit
            stable_for_ms=250
        }
        $script:Report.ime_conversion_candidate = $convertedPreedit
        $script:Report.ime_composition_invariants += 'Space: source, selection, dirty state, history, and profile unchanged'
        Add-Observation 'configured IME stable Space conversion' $converted

        $candidateMovementObserved = $false
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x28
        try {
            $moved = Wait-EditorRecoveryStableState -WaitMs 5000 -Description 'distinct stable candidate movement after Down' -Condition {
                param($value)
                Assert-EditorRecoveryImeInvariant $value $baseline 'candidate movement'
                [bool]$value.ime.active -and
                    -not [string]::IsNullOrEmpty([string]$value.ime.preedit) -and
                    [string]$value.ime.preedit -cne $convertedPreedit
            }
            $candidateMovementObserved = $true
            $state = $moved
            $script:Report.ime_candidate_movement = @{
                status='PASS'; key='Down'; before_preedit=$convertedPreedit
                after_preedit=[string]$moved.ime.preedit; stable_for_ms=250
                evidence='distinct stable owner-thread preedit after Down'
            }
            $script:Report.ime_composition_invariants += 'Down: source, selection, dirty state, history, and profile unchanged'
            Add-Observation 'configured IME candidate movement with distinct readback' $moved
        } catch {
            if ($_.Exception.Message -match '^OBSERVER_FAILURE:|^PRODUCT_ASSERTION:') { throw }
            $lastCandidateState = $_.Exception.Data['LastDiagnosticState']
            if ($null -eq $lastCandidateState) { throw }
            Assert-EditorRecoveryImeInvariant $lastCandidateState $baseline 'candidate movement'
            $script:Report.ime_candidate_movement = @{
                status='NOT_RUN'; key='Down'; before_preedit=$convertedPreedit
                after_preedit=[string]$lastCandidateState.ime.preedit
                reason='No distinct stable preedit was observed; candidate-list movement is not exposed by the diagnostic contract.'
            }
            $script:Report.ime_composition_invariants += 'Down attempt: source, selection, dirty state, history, and profile unchanged'
            $state = $lastCandidateState
        }

        $cancelResult = Cancel-EditorRecoveryImeComposition $baseline $state 'configured IME reading' 3
        $state = $cancelResult.state
        if (-not $cancelResult.cancelled) {
            $script:Report.ime_status = 'BLOCKED: native composition remained active after three ordinary Escape stages.'
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
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x20
        $converted = Wait-EditorRecoveryStableState -WaitMs 10000 -Description 'stable conversion before Enter commit' -Condition {
            param($value)
            Assert-EditorRecoveryImeInvariant $value $baseline 'commit Space conversion'
            [bool]$value.ime.active -and
                -not [string]::IsNullOrEmpty([string]$value.ime.preedit) -and
                [string]$value.ime.preedit -cne $rawCommitPreedit -and
                (Test-EditorRecoveryJapaneseText ([string]$value.ime.preedit))
        }
        $script:Report.ime_conversion_candidate = [string]$converted.ime.preedit
        $script:Report.ime_space_conversion.commit_preedit = [string]$converted.ime.preedit
        $script:Report.ime_space_conversion.commit_stable_for_ms = 250
        $script:Report.ime_composition_invariants += 'commit Space: source, selection, dirty state, history, and profile unchanged'
        Add-Observation 'configured IME stable conversion before commit' $converted

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
                [int]$value.redo_count -eq $baseRedoCount -and
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
                [int]$value.redo_count -eq ($baseRedoCount + 1)
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
                [int]$value.redo_count -eq $baseRedoCount
        } 'Redo configured IME commit'
        Add-Observation 'Redo configured IME commit' $state
        $movementLabel = if ($candidateMovementObserved) { 'candidate movement read back' } else { 'candidate movement NOT_RUN' }
        $script:Report.ime_status = if ($candidateMovementObserved) {
            "PASS: active configured profile '$profileDescription' (language $($profile.language_id)); full scripted reading, stable Space conversion, Down readback, Escape cancellation, Enter commit, and one-unit Undo/Redo were observed."
        } else {
            "PARTIAL: active configured profile '$profileDescription' (language $($profile.language_id)); full reading, stable conversion, Escape, commit, and Undo/Redo passed; $movementLabel."
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
    for ($attempt = 0; $attempt -lt 2; $attempt++) {
        $state = Wait-Diagnostic { param($value) $true } 'fresh diagnostic while selecting a fixture tab'
        if ($state.source -ceq $ExpectedSource) { return $state }
        if ([int]$state.tabs -ne 2) { throw 'The owned fixture process no longer has exactly two tabs.' }
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x09 ([uint16[]]@(0x11))
        $state = Wait-Diagnostic {
            param($value)
            $value.source -ceq $ExpectedSource
        } 'Ctrl+Tab switch to the requested synthetic fixture'
    }
    throw 'The requested synthetic fixture was not active after two bounded tab switches.'
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
    if ([MdliteRustAcceptanceNative]::GetSaveAsFilenameFocus(
        $Dialog, $script:WindowHandle, $script:ProcessId) -ne $Field) {
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
    Send-UnicodeText $Label $script:WindowHandle
    $count = Wait-UiListCount $script:WindowHandle 151 1 2500
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

function Invoke-EditorViewportBodyClick([string]$CaseId, [string]$Phase) {
    $before = Read-Diagnostic
    $parent = Get-UiParentRoot $before
    $expectedEditor = Convert-DiagnosticWindowHandle $parent.editor_hwnd
    $main = Convert-DiagnosticWindowHandle $parent.hwnd
    $compact = Convert-DiagnosticWindowHandle $parent.compact_hwnd
    Assert-EditorRecoveryMainIdentity $script:WindowHandle $main $expectedEditor 'before viewport click'
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
    Invoke-UiDiagnosticClick $x $y ([double]$parent.dpi)

    $after = Read-Diagnostic
    $afterParent = Get-UiParentRoot $after
    $afterMain = Convert-DiagnosticWindowHandle $afterParent.hwnd
    $afterEditor = Convert-DiagnosticWindowHandle $afterParent.editor_hwnd
    $afterCompact = Convert-DiagnosticWindowHandle $afterParent.compact_hwnd
    Assert-EditorRecoveryMainIdentity $expectedEditor $afterMain $afterEditor 'after viewport click'
    $focusAfter = [MdliteRustAcceptanceNative]::ObserveOwnedEditorFocus(
        $expectedEditor, $afterMain, $afterCompact, $script:ProcessId)
    $record = [ordered]@{
        case_id = $CaseId; phase = $Phase; observed_utc = [DateTime]::UtcNow.ToString('o')
        expected_window_kind = 'Main'
        click_viewport_dip = @{ x=$x; y=$y; dpi=[double]$parent.dpi }
        expected_editor_hwnd = '0x{0:X}' -f $expectedEditor.ToInt64()
        editor_hwnd_after_click = '0x{0:X}' -f $afterEditor.ToInt64()
        source_sha256_before = Get-UiTextSha256 ([string]$before.source)
        source_sha256_after = Get-UiTextSha256 ([string]$after.source)
        document_id_unchanged = ([long]$before.document_id -eq [long]$after.document_id)
        revision_unchanged = ([int]$before.revision -eq [int]$after.revision)
        dirty_unchanged = ([bool]$before.dirty -eq [bool]$after.dirty)
        history_unchanged = ([int]$before.undo_count -eq [int]$after.undo_count -and
            [int]$before.redo_count -eq [int]$after.redo_count)
        selection_before = $before.selection; selection_after = $after.selection
        native_focus_before = $focusBefore; native_focus_after = $focusAfter
        input_observed_ms = $null
    }
    $script:Report.native_editor_focus_checks += [pscustomobject]$record
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

function Invoke-UiDiagnosticDrag(
    [double]$StartDipX,
    [double]$StartDipY,
    [double]$EndDipX,
    [double]$EndDipY,
    [double]$DpiScale
) {
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
    Set-OwnedForeground -Window $script:WindowHandle
    $sent = [MdliteRustAcceptanceNative]::DragOwnedBetween(
        $script:WindowHandle, $script:ProcessId,
        $screenStartX, $screenStartY, $screenEndX, $screenEndY)
    if ($sent -ne 2) {
        throw [System.InvalidOperationException]::new('OWNED_DRAG_FAILED')
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
            Invoke-UiDiagnosticDrag $splitX $splitY ($splitX + $delta) $splitY ([double]$expandedParent.dpi)
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
    $script:EditorRecoveryWorkspaceSettingsPath = Join-Path $script:UiFixtureRoot '.mdlite/settings.toml'
    $script:EditorRecoveryLf = [string][char]10
    $script:EditorRecoveryCrLf = [string][char]13 + [string][char]10
    $script:EditorRecoveryWorkspaceSource = (@(
        '# Synthetic Editor Recovery fixture', '', 'alpha beta 日本語', '',
        '> quoted line', '', '- item one', '', '| first | second |',
        '| --- | --- |', '| cell | value |', '', '~~~text',
        'literal **Markdown**', '~~~', ''
    ) -join $script:EditorRecoveryCrLf)
    $workspaceSettings = "schema_version = 1`nauto_save = false`n"
    [IO.File]::WriteAllText($script:EditorRecoveryWorkspaceSettingsPath, $workspaceSettings, [Text.UTF8Encoding]::new($false))
    $script:Report.workspace_settings = [ordered]@{
        path = $script:EditorRecoveryWorkspaceSettingsPath.Substring($ProjectRoot.Length).TrimStart('\').Replace('\','/')
        scope = 'harness-owned workspace only'
        configured_auto_save = $false
        runtime_application_readback = 'NOT_EXPOSED; this file records fixture setup only.'
        sha256 = (Get-FileHash -LiteralPath $script:EditorRecoveryWorkspaceSettingsPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    [IO.File]::WriteAllText($script:EditorRecoveryBlankPath, '', [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($script:EditorRecoveryWorkspacePath, $script:EditorRecoveryWorkspaceSource, [Text.UTF8Encoding]::new($true))
    $script:EditorRecoveryFixturePaths = @($script:EditorRecoveryBlankPath, $script:EditorRecoveryWorkspacePath)
    $script:Report.fixtures = @(
        foreach ($fixture in $script:EditorRecoveryFixturePaths) {
            [pscustomobject]@{
                path = $fixture.Substring($ProjectRoot.Length).TrimStart('\').Replace('\', '/')
                initial_sha256 = (Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant()
                initial_bytes = [IO.File]::ReadAllBytes($fixture).Length
            }
        }
    )
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

function Test-EditorRecoveryProfileIdentity($Expected, $Actual) {
    if ($null -eq $Expected -or $null -eq $Actual) { return $false }
    foreach ($name in @('status','profile_type','language_id','clsid','profile_guid','hkl')) {
        $left = $Expected.PSObject.Properties[$name]
        $right = $Actual.PSObject.Properties[$name]
        if ($null -eq $left -or $null -eq $right -or [string]$left.Value -cne [string]$right.Value) { return $false }
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
        available_profiles=@(); selected_profile=$null; hresult=$null; fallback_used=$false
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
        $requestRecord.hresult = '0x{0:X8}' -f ([uint32]$hresult)
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
                    $matchesReadback = if ($ExpectedIdentity) {
                        Test-EditorRecoveryProfileIdentity $ExpectedIdentity $candidate.active_input_profile
                    } else {
                        $actual = $candidate.active_input_profile
                        $null -ne $actual -and [string]$actual.status -ceq 'active' -and
                        [string]$actual.profile_type -ceq [string]$expected.profile_type -and
                        [string]$actual.language_id -ceq [string]$expected.language_id -and
                        [string]$actual.clsid -ieq [string]$expected.clsid -and
                        [string]$actual.profile_guid -ieq [string]$expected.profile_guid
                    }
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
        $requestRecord.api_error = 'ITfInputProcessorProfileMgr::ActivateProfile returned 0x{0:X8}.' -f ([uint32]$hresult)
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

function Restore-EditorRecoveryInitialImeStateForFailure {
    if ($script:Report.ime_profile_restoration -and $script:Report.ime_profile_restoration.status -eq 'PASS') {
        return $script:Report.ime_profile_restoration
    }
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
            $cancelResult = Cancel-EditorRecoveryImeComposition $before $before 'failure-finally owned composition' 3
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
    if ($null -eq $script:Process) { return }
    $script:Process.Refresh()
    if (-not $script:Process.HasExited) {
        Assert-OwnedWindow $script:WindowHandle
        $result = [IntPtr]::Zero
        $sent = [MdliteRustAcceptanceNative]::SendMessageTimeoutW(
            $script:WindowHandle,[uint32]$WM_CLOSE,[IntPtr]::Zero,[IntPtr]::Zero,
            [uint32]$SMTO_ABORTIFHUNG,[uint32]1000,[ref]$result)
        if (-not $sent -or -not $script:Process.WaitForExit(15000)) {
            throw ('Owned {0} process did not close normally; it remains open for inspection.' -f $Label)
        }
    }
    $script:Report.normal_close += [pscustomobject]@{ stage=$Label; status='PASS'; pid=[int]$script:Process.Id }
    $script:Process.Dispose(); $script:Process=$null
    $script:WindowHandle=[IntPtr]::Zero; $script:InputWindowHandle=[IntPtr]::Zero; $script:ProcessId=[uint32]0
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
        E06='Pointer, drag, Shift+click, and grapheme replacement are not automated in this bounded mode.'
        E07='List/task/quote/code Tab and Shift+Tab continuation rules remain untested.'
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
    $script:Report.input_delivery = 'Editor ASCII uses guarded current-layout virtual-key SendInput; IME uses selected installed profiles. Owned SaveAs filename fixture setup uses bounded WM_SETTEXT and exact WM_GETTEXT, not editor keyboard proof. Unicode packets and OS clipboard are not used.'
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
        Start-UiParityProcess @($script:EditorRecoveryWorkspacePath,$script:EditorRecoveryBlankPath) $script:DiagnosticPath
        [void](Wait-Diagnostic { param($s) [int]$s.tabs -eq 2 } 'two synthetic Workspace fixtures')
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
        [void](Invoke-EditorRecoveryImeCase 'E03' 'ATOK')
        $script:CurrentEditorRecoveryCaseId='E04'
        [void](Invoke-EditorRecoveryImeCase 'E04' 'Microsoft IME')
        $state=Read-Diagnostic
        if ($state.source -cne $script:EditorRecoveryExpectedSource) { throw 'PRODUCT_ASSERTION: E04 source diverged from the exact committed IME suffixes.' }

        $script:CurrentEditorRecoveryCaseId='E04'
        $profileState=Read-Diagnostic
        $caseE04=@($script:Report.editor_recovery_cases | Where-Object {$_.id -eq 'E04'})[0]
        $coreSequenceStatusE04 = if ($caseE04.details -is [System.Collections.IDictionary] -and $caseE04.details.Contains('core_sequence_status')) {
            [string]$caseE04.details['core_sequence_status']
        } else { '' }
        if ($coreSequenceStatusE04 -in @('PASS','PARTIAL') -and
            [string]$profileState.active_input_profile.description -match 'Microsoft IME' -and
            [bool]$profileState.ime.open_status.available) {
            $originalOpen=[bool]$profileState.ime.open_status.open
            if (-not $originalOpen) {
                Send-Key $script:InputWindowHandle 0x16
                $profileState=Wait-ImeOpenState $true ([string]$profileState.source)
            }
            $longRomaji='nihongokyouhaimehenka'
            $longExpectedReading='にほんごきょうはいめへんか'
            $longBaseline=$profileState
            $script:ImeEscapesSentForComposition=0
            Send-Ascii $longRomaji $script:InputWindowHandle
            try {
                $longState=Wait-EditorRecoveryStableState -WaitMs 10000 -Description 'E04 complete Microsoft IME reading' -Condition {
                    param($s)
                    Assert-EditorRecoveryImeInvariant $s $longBaseline 'E04 long reading'
                    [bool]$s.ime.active -and [string]$s.ime.preedit -ceq $longExpectedReading
                }
            } catch {
                if ($_.Exception.Message -match '^OBSERVER_FAILURE:|^PRODUCT_ASSERTION:') { throw }
                $observedLongState=$_.Exception.Data['LastDiagnosticState']
                if ($null -eq $observedLongState) { throw }
                $caseE04.details.long_reading_status='NOT_RUN'
                $caseE04.details.long_expected_reading=$longExpectedReading
                $caseE04.details.long_actual_reading=[string]$observedLongState.ime.preedit
                throw ('ENVIRONMENT_BLOCKED: E04 complete raw reading was not observed; expected=' + $longExpectedReading + '; actual=' + [string]$observedLongState.ime.preedit)
            }
            if ([string]$longState.ime.preedit -cne $longExpectedReading) {
                throw ('ENVIRONMENT_BLOCKED: E04 complete raw reading differed; expected=' + $longExpectedReading + '; actual=' + [string]$longState.ime.preedit)
            }
            [void](Save-EditorRecoveryEvidence 'E04' 'long-preedit')
            $longRawPreedit=[string]$longState.ime.preedit
            Send-Key $script:InputWindowHandle 0x20
            $converted=Wait-EditorRecoveryStableState -WaitMs 10000 -Description 'E04 stable Microsoft IME conversion' -Condition {
                param($s)
                Assert-EditorRecoveryImeInvariant $s $longBaseline 'E04 long conversion'
                [bool]$s.ime.active -and [string]$s.ime.preedit -cne $longRawPreedit -and
                    (Test-EditorRecoveryJapaneseText ([string]$s.ime.preedit))
            }
            [void](Save-EditorRecoveryEvidence 'E04' 'long-conversion')
            $longCancelResult=Cancel-EditorRecoveryImeComposition $longBaseline $converted 'E04 long conversion' 3
            $longCancelled=$longCancelResult.state
            if (-not $longCancelResult.cancelled) { throw 'ENVIRONMENT_BLOCKED: E04 long composition remained active after three ordinary Escape stages.' }
            [void](Assert-EditorRecoveryDocument $script:EditorRecoveryExpectedSource ([Text.Encoding]::UTF8.GetByteCount($script:EditorRecoveryExpectedSource)) $true 'E04 Esc leaves exact source unchanged')
            $script:ImeProfile=$profileState.active_input_profile
            $script:ImeInitialOpen=$originalOpen
            $script:ImeRestorePending=$true; $script:ImeRestoreAttempted=$false
            Restore-ImeOpenState
            $caseE04.details.long_preedit_length=([string]$longState.ime.preedit).Length
            $caseE04.details.long_reading=$longRomaji
            $caseE04.details.long_expected_reading=$longExpectedReading
            $caseE04.details.long_actual_reading=[string]$longState.ime.preedit
            $caseE04.details.long_reading_status='PASS'
            $caseE04.details.long_converted_preedit=[string]$converted.ime.preedit
            $caseE04.details.long_conversion_stable_for_ms=250
            $caseE04.details.long_conversion_source_unchanged=($converted.source -ceq $script:EditorRecoveryExpectedSource)
            $caseE04.details.long_cancel_source_unchanged=($longCancelled.source -ceq $script:EditorRecoveryExpectedSource)
            $caseE04.details.long_escape_stages=@($longCancelResult.stages)
        }
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
        $state=Wait-Diagnostic {param($s)$s.source -ceq $source -and [int]$s.selection.anchor -eq 0 -and [int]$s.selection.head -ge 5 -and [int]$s.selection.head -le 6} 'E05 Ctrl+Shift+Right'
        $bytes=[Text.Encoding]::UTF8.GetBytes($source)
        $selected=[Text.Encoding]::UTF8.GetString($bytes,[int]$state.selection.anchor,[int]$state.selection.head-[int]$state.selection.anchor)
        if (-not $selected.StartsWith('alpha',[StringComparison]::Ordinal)) { throw 'PRODUCT_ASSERTION: E05 selected the wrong source word.' }
        Send-Key $script:InputWindowHandle 0x25
        [void](Wait-Diagnostic {param($s)$s.source -ceq $source -and [int]$s.selection.anchor -eq 0 -and [int]$s.selection.head -eq 0} 'E05 Left collapse')
        $end=[Text.Encoding]::UTF8.GetByteCount($source)
        Send-Key $script:InputWindowHandle 0x23 ([uint16[]]@(0x11))
        [void](Assert-EditorRecoveryDocument $source $end $true 'E05 Ctrl+End')
        [void](Save-EditorRecoveryEvidence 'E05' 'movement-selection')
        Set-EditorRecoveryCase 'E05' 'NOT_RUN' @{
            verified=@('Ctrl+Home','Shift+Right','Ctrl+Shift+Right','Left','Ctrl+End')
            not_run=@('remaining navigation commands','selection replacement and Undo branch')
            physical_typematic='NOT_RUN'
            reason='Listed navigation substeps are recorded; the full movement and selection-replacement row remains untested.'
        }

        $script:CurrentEditorRecoveryCaseId='E09'
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

        $script:CurrentEditorRecoveryCaseId='E13'
        $saved=[string](Read-Diagnostic).source
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
        [void](Switch-ToSource $saved); Save-ActiveFixture $saved $script:EditorRecoveryBlankPath
        [void](Save-EditorRecoveryEvidence 'E13' 'pre-close-source')
        Close-EditorRecoveryProcess 'continuous-workspace-session'
        $script:UiFixtureRoot=Join-Path $script:UiRunDirectory 'workspace'
        Start-UiParityProcess @($script:EditorRecoveryWorkspacePath,$script:EditorRecoveryBlankPath) $script:DiagnosticPath
        [void](Wait-Diagnostic {param($s)[int]$s.tabs -eq 2} 'E13 reopen fixtures')
        [void](Switch-ToSource $saved)
        [void](Assert-EditorRecoveryDocument $saved ([Text.Encoding]::UTF8.GetByteCount($saved)) $false 'E13 reopened exact blank source')
        [void](Switch-ToSource $structuredExpected)
        [void](Assert-EditorRecoveryDocument $structuredExpected ([Text.Encoding]::UTF8.GetByteCount($structuredExpected)) $false 'E13 reopened exact BOM/CRLF source')
        [void](Save-EditorRecoveryEvidence 'E13' 'reopened-source')
        Close-EditorRecoveryProcess 'reopen-verification'
        Set-EditorRecoveryCase 'E13' 'NOT_RUN' @{save_bytes='PASS';reopen='PASS';normal_close='PASS';autosave='NOT_RUN';reason='Autosave and recovery checkpoint timing remain untested.'}

        $script:Report.operations += [pscustomobject]@{ name='continuous EditorRecovery core sequence';cases=@('E01','E02','E03','E04','E05','E09','E11','E13') }
    } catch {
        $failureMessage = $_.Exception.Message
        $script:Report.failure=$failureMessage
        $script:Report.failure_kind = if ($failureMessage -match '^(OBSERVER_FAILURE|SEMANTIC_WAIT_TIMEOUT|SETUP_FAILURE|ENVIRONMENT_BLOCKED):') {
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
        if ($null -eq $previousImeRestoration -or $previousImeRestoration.status -cne 'PASS') {
            if ($previousImeRestoration -and $finalImeRestoration.status -eq 'PASS') {
                $finalImeRestoration.previous_attempt = $previousImeRestoration
            }
            $script:Report.ime_profile_restoration = $finalImeRestoration
        }
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
