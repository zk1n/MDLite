#Requires -Version 5.1
[CmdletBinding()]
param([ValidateSet('debug','release')][string]$Preset = 'debug', [string]$OutputPath, [string]$CaseFilter = '.*')

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') { throw 'Run with powershell.exe -STA -File.' }
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$repoRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $repoRoot "build/$Preset/MDLite.exe"
if (-not $OutputPath) { $OutputPath = Join-Path $repoRoot "build/verification/prehuman-edit/edit-history-$Preset.json" }
$receiptPath = Join-Path $repoRoot "build/verification/build-receipt-$Preset.json"
$receipt = Get-Content -Raw -LiteralPath $receiptPath | ConvertFrom-Json
$exeHash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
if ($receipt.build_status -ne 'PASS' -or $receipt.executable_sha256 -ne $exeHash) { throw 'Build receipt/executable mismatch.' }

function Get-SourceHash {
    $prefix = [IO.Path]::GetFullPath($repoRoot).TrimEnd([char[]]@('\','/')) + [IO.Path]::DirectorySeparatorChar
    [string[]]$paths = @((Get-ChildItem (Join-Path $repoRoot 'src') -Recurse -File).FullName) + @((Join-Path $repoRoot 'CMakeLists.txt'), (Join-Path $repoRoot 'CMakePresets.json'))
    [Array]::Sort($paths, [StringComparer]::Ordinal)
    $manifest = foreach ($path in $paths) { '{0}:{1}' -f $path.Substring($prefix.Length).Replace('\','/'), (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
    $hash = [Security.Cryptography.SHA256]::Create()
    try { [BitConverter]::ToString($hash.ComputeHash([Text.Encoding]::UTF8.GetBytes([string]::Join("`n", [string[]]$manifest)))).Replace('-','').ToLowerInvariant() } finally { $hash.Dispose() }
}
$sourceHash = Get-SourceHash
if ($receipt.source_sha256 -ne $sourceHash) { throw 'Build receipt/source mismatch.' }
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class MDLiteEditNative {
    public delegate bool EnumProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", SetLastError=true)]
    public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr wp, IntPtr lp, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern IntPtr SendMessageTimeoutText(IntPtr window, uint message, IntPtr wp, System.Text.StringBuilder text, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern bool InvalidateRect(IntPtr window, IntPtr rect, bool erase);
    [DllImport("user32.dll")] public static extern bool UpdateWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct GUIINFO { public int Size; public uint Flags; public IntPtr Active,Focus,Capture,Menu,MoveSize,Caret; public RECT CaretRect; }
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint thread, ref GUIINFO info);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] public static extern bool OpenClipboard(IntPtr owner);
    [DllImport("user32.dll")] public static extern bool CloseClipboard();
    [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint format);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr data);
    [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr data);
    [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr data);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint flags, UIntPtr size);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalFree(IntPtr data);
    [DllImport("user32.dll")] static extern bool EmptyClipboard();
    [DllImport("user32.dll")] static extern IntPtr SetClipboardData(uint format, IntPtr data);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr CreateWindowEx(uint extended,string cls,string title,uint style,int x,int y,int width,int height,IntPtr parent,IntPtr menu,IntPtr instance,IntPtr parameter);
    [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetOpenClipboardWindow();
    [DllImport("user32.dll")] static extern IntPtr GetClipboardOwner();
    [DllImport("user32.dll")] static extern bool IsClipboardFormatAvailable(uint format);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr window,System.Text.StringBuilder name,int capacity);
    static string ClipboardDispatchState() {
        uint openPid=0,ownerPid=0;IntPtr open=GetOpenClipboardWindow(),owner=GetClipboardOwner();
        GetWindowThreadProcessId(open,out openPid);GetWindowThreadProcessId(owner,out ownerPid);
        bool available=IsClipboardFormatAvailable(13),opened=OpenClipboard(IntPtr.Zero);if(opened)CloseClipboard();
        return "open_hwnd="+open+" open_pid="+openPid+" owner_hwnd="+owner+" owner_pid="+ownerPid+" unicode="+available+" open_probe="+opened;
    }
    public static string DispatchPasteObservation(IntPtr editor) {
        string before=ClipboardDispatchState();IntPtr result;IntPtr api=SendMessageTimeout(editor,0x302,IntPtr.Zero,IntPtr.Zero,2,5000,out result);int error=Marshal.GetLastWin32Error();
        return "before: "+before+"; api="+api+" reply="+result+" error="+error+"; after: "+ClipboardDispatchState();
    }
    public class CutClipboardObservation { public bool Available,FixtureOwner;public uint Sequence,OwnerPid;public IntPtr Owner; }
    public static CutClipboardObservation ObserveCutClipboard(IntPtr editor,uint fixturePid) {
        var observation=new CutClipboardObservation();uint editorPid;GetWindowThreadProcessId(editor,out editorPid);
        if(fixturePid==0||editorPid!=fixturePid||!OpenClipboard(IntPtr.Zero))return observation;
        try {
            observation.Available=true;observation.Sequence=GetClipboardSequenceNumber();
            observation.Owner=GetClipboardOwner();GetWindowThreadProcessId(observation.Owner,out observation.OwnerPid);
            observation.FixtureOwner=observation.Owner!=IntPtr.Zero&&observation.OwnerPid==fixturePid;
        } finally {CloseClipboard();}
        return observation;
    }
    static IntPtr clipboardOwner;
    public static bool FixtureClipboardMutated;
    public static uint FixtureClipboardMutationSequence;
    public static void SetFixtureClipboardText(string text) {
        FixtureClipboardMutated=false;
        FixtureClipboardMutationSequence=0;
        if(clipboardOwner==IntPtr.Zero)clipboardOwner=CreateWindowEx(0,"STATIC","MDLite clipboard fixture",0x80000000,0,0,0,0,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero);
        if(clipboardOwner==IntPtr.Zero)throw new InvalidOperationException("Clipboard fixture owner could not be created");
        IntPtr data=GlobalAlloc(2,(UIntPtr)((ulong)(text.Length+1)*2));if(data==IntPtr.Zero)throw new InvalidOperationException("Clipboard fixture allocation failed");
        IntPtr pointer=GlobalLock(data);if(pointer==IntPtr.Zero){GlobalFree(data);throw new InvalidOperationException("Clipboard fixture text could not be locked");}
        Marshal.Copy(text.ToCharArray(),0,pointer,text.Length);Marshal.WriteInt16(pointer,text.Length*2,0);GlobalUnlock(data);
        if(!OpenClipboard(clipboardOwner)){GlobalFree(data);throw new InvalidOperationException("Clipboard fixture publication blocked; open_owner="+GetOpenClipboardWindow());}
        bool published=false;
        try {
            if(EmptyClipboard()) {
                FixtureClipboardMutated=true;
                FixtureClipboardMutationSequence=GetClipboardSequenceNumber();
                published=SetClipboardData(13,data)!=IntPtr.Zero;
                FixtureClipboardMutationSequence=GetClipboardSequenceNumber();
            }
        } finally {CloseClipboard();if(!published)GlobalFree(data);}
        if(!published)throw new InvalidOperationException("Clipboard fixture text publication failed");
    }
    public static void CloseFixtureClipboardOwner(){if(clipboardOwner!=IntPtr.Zero){DestroyWindow(clipboardOwner);clipboardOwner=IntPtr.Zero;}}
    public static string ClipboardText() {
        if(!OpenClipboard(IntPtr.Zero))throw new InvalidOperationException("Clipboard inspection could not open it");
        IntPtr data=IntPtr.Zero;IntPtr pointer=IntPtr.Zero;
        try{data=GetClipboardData(13);if(data==IntPtr.Zero)throw new InvalidOperationException("Clipboard Unicode text unavailable");pointer=GlobalLock(data);if(pointer==IntPtr.Zero)throw new InvalidOperationException("Clipboard inspection could not lock text");ulong capacity=GlobalSize(data).ToUInt64()/2;int length=0;while((ulong)length<capacity&&Marshal.ReadInt16(pointer,length*2)!=0)length++;if((ulong)length>=capacity)throw new InvalidOperationException("Clipboard text is not terminated");return Marshal.PtrToStringUni(pointer,length);}
        finally{if(pointer!=IntPtr.Zero)GlobalUnlock(data);CloseClipboard();}
    }
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
    [DllImport("kernel32.dll")] static extern IntPtr VirtualAllocEx(IntPtr process,IntPtr address,UIntPtr size,uint type,uint protection);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr process,IntPtr address,UIntPtr size,uint type);
    [DllImport("kernel32.dll")] static extern bool WriteProcessMemory(IntPtr process,IntPtr address,byte[] data,UIntPtr size,out UIntPtr written);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr process,IntPtr address,byte[] data,UIntPtr size,out UIntPtr read);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    public static RECT TreeRect(IntPtr window,IntPtr item) {
        uint pid;GetWindowThreadProcessId(window,out pid);IntPtr process=OpenProcess(0x38,false,pid);if(process==IntPtr.Zero)throw new InvalidOperationException("OpenProcess tree inspection failed");
        IntPtr memory=IntPtr.Zero;
        try { memory=VirtualAllocEx(process,IntPtr.Zero,(UIntPtr)16,0x3000,4);if(memory==IntPtr.Zero)throw new InvalidOperationException("Tree rectangle allocation failed");
            byte[] data=new byte[16];Array.Copy(BitConverter.GetBytes(item.ToInt64()),data,IntPtr.Size);UIntPtr count;IntPtr result;
            if(!WriteProcessMemory(process,memory,data,(UIntPtr)16,out count)||SendMessageTimeout(window,0x1104,(IntPtr)1,memory,2,5000,out result)==IntPtr.Zero||result==IntPtr.Zero||!ReadProcessMemory(process,memory,data,(UIntPtr)16,out count))throw new InvalidOperationException("Native tree item rectangle unavailable");
            return new RECT{Left=BitConverter.ToInt32(data,0),Top=BitConverter.ToInt32(data,4),Right=BitConverter.ToInt32(data,8),Bottom=BitConverter.ToInt32(data,12)};
        } finally {if(memory!=IntPtr.Zero)VirtualFreeEx(process,memory,UIntPtr.Zero,0x8000);CloseHandle(process);}
    }
    public static void BeginOutlineSyntheticDrag(IntPtr main,IntPtr outline,IntPtr item,long source,uint ownedPid) {
        uint mainPid,outlinePid;GetWindowThreadProcessId(main,out mainPid);GetWindowThreadProcessId(outline,out outlinePid);
        if(mainPid!=ownedPid||outlinePid!=ownedPid)throw new InvalidOperationException("Outline notification owner mismatch");
        IntPtr process=OpenProcess(0x38,false,ownedPid);if(process==IntPtr.Zero)throw new InvalidOperationException("Outline fixture process access failed");IntPtr memory=IntPtr.Zero;
        try{memory=VirtualAllocEx(process,IntPtr.Zero,(UIntPtr)152,0x3000,4);if(memory==IntPtr.Zero)throw new InvalidOperationException("Outline notification allocation failed");
            byte[] data=new byte[152];Array.Copy(BitConverter.GetBytes(outline.ToInt64()),0,data,0,8);Array.Copy(BitConverter.GetBytes((long)103),0,data,8,8);Array.Copy(BitConverter.GetBytes(unchecked((uint)-456)),0,data,16,4);Array.Copy(BitConverter.GetBytes((uint)4),0,data,88,4);Array.Copy(BitConverter.GetBytes(item.ToInt64()),0,data,96,8);Array.Copy(BitConverter.GetBytes(source),0,data,136,8);UIntPtr count;IntPtr result;
            if(!WriteProcessMemory(process,memory,data,(UIntPtr)152,out count)||SendMessageTimeout(main,0x004e,(IntPtr)103,memory,2,5000,out result)==IntPtr.Zero)throw new InvalidOperationException("Synthetic outline notification delivery failed; Win32="+Marshal.GetLastWin32Error());
        }finally{if(memory!=IntPtr.Zero)VirtualFreeEx(process,memory,UIntPtr.Zero,0x8000);CloseHandle(process);}
    }
    public static GUIINFO CaretInfo(IntPtr window) { uint pid; uint thread=GetWindowThreadProcessId(window,out pid); var info=new GUIINFO(); info.Size=Marshal.SizeOf(info); if(!GetGUIThreadInfo(thread,ref info)) throw new InvalidOperationException("GetGUIThreadInfo failed"); return info; }
}
'@
function Send([IntPtr]$Window, [uint32]$Message, [long]$Wp = 0, [long]$Lp = 0) {
    $result = [IntPtr]::Zero
    if ([MDLiteEditNative]::SendMessageTimeout($Window,$Message,[IntPtr]$Wp,[IntPtr]$Lp,2,5000,[ref]$result) -eq [IntPtr]::Zero) {
        throw "Native message timed out or failed: hwnd=$Window message=$Message error=$([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
    }
    $result.ToInt64()
}
function Publish-FixtureClipboard([string]$Text) {
    try { [MDLiteEditNative]::SetFixtureClipboardText($Text) }
    finally {
        if ([MDLiteEditNative]::FixtureClipboardMutated) {
            $script:clipboardChanged=$true
            $script:lastTestClipboardSequence=[MDLiteEditNative]::FixtureClipboardMutationSequence
        }
    }
}
function Cut-FixtureClipboard([IntPtr]$Editor,[uint32]$FixturePid) {
    $before=[MDLiteEditNative]::ObserveCutClipboard($Editor,$FixturePid)
    if (-not $before.Available) {
        $script:clipboardTrackingUnconfirmed=$true
        throw 'Cut clipboard tracking unavailable before dispatch; cut not sent.'
    }
    try { [void](Send $Editor 0x0300) }
    finally {
        $after=[MDLiteEditNative]::ObserveCutClipboard($Editor,$FixturePid)
        if (-not $before.Available -or -not $after.Available) {
            $script:clipboardTrackingUnconfirmed=$true
        } elseif ($after.Sequence -ne $before.Sequence) {
            if ($after.FixtureOwner) {
                $script:clipboardChanged=$true
                $script:lastTestClipboardSequence=$after.Sequence
            } else { $script:clipboardExternalGeneration=$true }
        }
    }
}
function Find-Control([IntPtr]$Main, [int]$Id) {
    $script:found = [IntPtr]::Zero
    $callback = [MDLiteEditNative+EnumProc]{ param($window,$parameter)
        if ([MDLiteEditNative]::GetDlgCtrlID($window) -eq $Id -and [MDLiteEditNative]::IsWindowVisible($window)) { $script:found=$window; return $false }; return $true
    }
    [void][MDLiteEditNative]::EnumChildWindows($Main,$callback,[IntPtr]::Zero)
    $script:found
}
function Native-Text([IntPtr]$Editor) {
    $length=Send $Editor 0x000e;$text=[Text.StringBuilder]::new([int]$length+1);$result=[IntPtr]::Zero
    if([MDLiteEditNative]::SendMessageTimeoutText($Editor,0x000d,[IntPtr]($length+1),$text,2,5000,[ref]$result) -eq [IntPtr]::Zero){throw 'Native text inspection failed.'}
    $text.ToString()
}
function Wait-Ready([IntPtr]$Main) {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $ready = Send $Main 0x8036
        if (($ready -band 5) -eq 5) { return }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Editor source/presentation not ready: $ready"
}
function State([IntPtr]$Main) {
    Wait-Ready $Main
    $length = Send $Main 0x8045 4
    if ($length -lt 0) { throw 'Document-state test probe unavailable.' }
    $text = [Text.StringBuilder]::new([int]$length)
    for ($i=0; $i -lt $length; $i++) { [void]$text.Append([char](Send $Main 0x8045 5 $i)) }
    [pscustomobject]@{ source=$text.ToString(); dirty=[bool](Send $Main 0x8045 0); undo=(Send $Main 0x8045 1); redo=(Send $Main 0x8045 2); revision=(Send $Main 0x8045 3); saved_revision=(Send $Main 0x8045 6); anchor=(Send $Main 0x8033); active=(Send $Main 0x8034); utc=[DateTime]::UtcNow.ToString('o') }
}
function Select-Source([IntPtr]$Main,[int]$Anchor,[int]$Active=$Anchor) { if ((Send $Main 0x8032 $Anchor $Active) -ne 1) { throw 'Selection probe failed.' } }
function Check([bool]$Condition,[string]$Expectation) {
    $script:assertions.Add([pscustomobject]@{expected=$Expectation;observed_pass=$Condition})
    if (-not $Condition) { $script:errors.Add($Expectation) }
}
function Save-Frame([IntPtr]$Window,[string]$Path) {
    $rect=[MDLiteEditNative+RECT]::new()
    if(-not [MDLiteEditNative]::GetWindowRect($Window,[ref]$rect)){throw 'Frame window rectangle unavailable.'}
    $bitmap=[Drawing.Bitmap]::new($rect.Right-$rect.Left,$rect.Bottom-$rect.Top)
    $graphics=[Drawing.Graphics]::FromImage($bitmap);$hdc=$graphics.GetHdc()
    try { $printed=[MDLiteEditNative]::PrintWindow($Window,$hdc,2) } finally { $graphics.ReleaseHdc($hdc);$graphics.Dispose() }
    try { if(-not $printed){throw 'PrintWindow frame unavailable.'};$bitmap.Save($Path,[Drawing.Imaging.ImageFormat]::Png) } finally { $bitmap.Dispose() }
}
function RoundTrip([IntPtr]$Main,[IntPtr]$Editor,$Before,[string]$Expected,[int]$Caret=-1) {
    $edited=State $Main
    Check ($edited.source -ceq $Expected) 'Edit source equals exact expected UTF16/CRLF content'
    Check ($edited.undo -eq $Before.undo+1 -and $edited.redo -eq 0) 'One operation adds exactly one history transaction'
    Check $edited.dirty 'Edit marks source Dirty'
    if ($Caret -ge 0) { Check ($edited.anchor -eq $Caret -and $edited.active -eq $Caret) 'Edit collapses caret to expected source offset' }
    [void](Send $Editor 0x0304)
    $undone=State $Main
    Check ($undone.source -ceq $Before.source -and $undone.dirty -eq $Before.dirty -and $undone.undo -eq $Before.undo -and $undone.redo -eq 1) 'Undo restores exact source, Dirty checkpoint and history'
    Check ($undone.anchor -eq $Before.anchor -and $undone.active -eq $Before.active) 'Undo restores anchor/active and selection direction'
    [void](Send $Editor 0x0454)
    $redone=State $Main
    Check ($redone.source -ceq $edited.source -and $redone.anchor -eq $edited.anchor -and $redone.active -eq $edited.active -and $redone.undo -eq $edited.undo -and $redone.redo -eq 0) 'Redo restores exact source, caret and one history transaction'
    [pscustomobject]@{ before=$Before; expected_source=$Expected; expected_caret=$Caret; edited=$edited; undone=$undone; redone=$redone }
}
$caseRoot = Join-Path (Split-Path -Parent $OutputPath) ('fixtures-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($caseRoot) | Out-Null
$cases=[Collections.Generic.List[object]]::new()
$oldSilent=$env:MDLITE_TEST_SILENT
$oldCrash=$env:MDLITE_TEST_NO_CRASH_UI
$env:MDLITE_TEST_SILENT='1'; $env:MDLITE_TEST_NO_CRASH_UI='1'
$clipboard=$null
$originalClipboard=[Windows.Forms.Clipboard]::GetDataObject()
$clipboardChanged=$false
$clipboardTrackingUnconfirmed=$false
$clipboardExternalGeneration=$false
$lastTestClipboardSequence=[MDLiteEditNative]::GetClipboardSequenceNumber()
if($null -ne $originalClipboard){
    $clipboard=[Windows.Forms.DataObject]::new()
    foreach($format in $originalClipboard.GetFormats($false)){
        $data=$originalClipboard.GetData($format,$false)
        if($null -ne $data){$clipboard.SetData($format,$false,$data)}
    }
}
function Run-Case([string]$Name,[string]$Source,[scriptblock]$CaseBody) {
    if ($Name -notmatch $CaseFilter) { return }
    $workspace=Join-Path $caseRoot $Name
    [IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $workspace '.mdlite/settings.toml'),"schema_version = 1`nauto_save = false`ntheme = `"dark`"`n",[Text.UTF8Encoding]::new($false))
    $path=Join-Path $workspace 'fixture.md'
    [IO.File]::WriteAllText($path,$Source,[Text.UTF8Encoding]::new($false))
    $process=$null; $main=[IntPtr]::Zero
    $script:errors=[Collections.Generic.List[string]]::new()
    $script:assertions=[Collections.Generic.List[object]]::new()
    $observation=$null
    try {
        $process=Start-Process -FilePath $executable -ArgumentList @($path) -WindowStyle Normal -PassThru
        $deadline=[DateTime]::UtcNow.AddSeconds(10)
        do { $process.Refresh(); $main=$process.MainWindowHandle; if ($main -ne [IntPtr]::Zero) { break }; if ($process.HasExited) { throw "Process exited: $($process.ExitCode)" }; Start-Sleep -Milliseconds 25 } while ([DateTime]::UtcNow -lt $deadline)
        if ($main -eq [IntPtr]::Zero) { throw "No main window for pid $($process.Id)" }
        [void][MDLiteEditNative]::SetWindowPos($main,[IntPtr]::Zero,0,0,1280,900,0x0004)
        Wait-Ready $main
        $editor=Find-Control $main 102
        if ($editor -eq [IntPtr]::Zero) { throw 'Visible RichEdit control absent.' }
        $initial=State $main
        Check ($initial.source -ceq $Source -and -not $initial.dirty -and $initial.undo -eq 0 -and $initial.redo -eq 0) 'Loaded fixture starts exact, clean and without history'
        $observation=& $CaseBody $main $editor $path
        $references=if($Name -like 'timed-*'){@('E10')}elseif($Name -like 'document-boundary-*'){@('E11')}elseif($Name -like 'profile-*'){@('E07','E04')}elseif($Name -like 'outline-*'){@('E07','E15')}elseif($Name -like 'empty-cell-*'){@('E09')}elseif($Name -match 'invariants|independent-documents'){@('E08')}else{@('E07')}
        $cases.Add([pscustomobject]@{ name=$Name; references=$references; lane='synthetic native-message'; clipboard_operation=([bool]($Name -match '^(?:table-)?(?:paste|cut)-direction-|^clipboard-busy-')); status=if($errors.Count -eq 0){'PASS'}else{'FAIL'}; fixture=$path; input='SendMessageTimeout Win32 messages to product RichEdit/commands; scalar state probes only for readback and selection setup'; assertions=@($assertions); failed_expectations=@($errors); observation=$observation; owner_pid=$process.Id; timestamp_utc=[DateTime]::UtcNow.ToString('o') })
    } catch { $cases.Add([pscustomobject]@{name=$Name;status='BLOCKED';reason=$_.Exception.Message;fixture=$path;observation=$observation;errors=@($errors)}) }
    finally {
        if ($null -ne $process) { if (-not $process.HasExited) { $process.Kill(); [void]$process.WaitForExit(3000) }; $process.Dispose() }
        $last=$cases[$cases.Count-1]
        [IO.File]::AppendAllText((Join-Path $caseRoot 'cases.ndjson'),($last|ConvertTo-Json -Depth 12 -Compress)+"`n",[Text.UTF8Encoding]::new($false))
        Write-Host "$Name $($last.status)"
    }
}
try {
    $body="本文 日本語 " + [char]0xd83d + [char]0xde00 + " e" + [char]0x301 + "`r`n次行 text`r`n末尾`r`n"
    $replacement="貼付 " + [char]0xd83d + [char]0xde80 + " a" + [char]0x301 + "`r`n二行"
    foreach ($reverse in @($false,$true)) {
        foreach ($operation in @('paste','cut','clear','delete','backspace','replace','enter')) {
            Run-Case "$operation-direction-$reverse" $body {
                param($main,$editor,$path)
                $begin=$body.IndexOf('日本語'); $end=$body.IndexOf('末尾')
                if ($reverse) { Select-Source $main $end $begin } else { Select-Source $main $begin $end }
                $before=State $main
                $nativeSelectionBefore=Send $editor 0x00b0
                $nativeBefore=Native-Text $editor
                $insert=if($operation -eq 'paste'){$replacement}elseif($operation -eq 'replace'){'X'}elseif($operation -eq 'enter'){"`r`n"}else{''}
                $expected=$body.Substring(0,$begin)+$insert+$body.Substring($end)
                switch($operation) {
                    paste { Publish-FixtureClipboard $replacement;Check ([MDLiteEditNative]::ClipboardText() -ceq $replacement) 'Clipboard publishes exact paste payload'; $pasteDispatch=[MDLiteEditNative]::DispatchPasteObservation($editor) }
                    cut { Cut-FixtureClipboard $editor ([uint32]$process.Id);Check ([MDLiteEditNative]::ClipboardText() -ceq $body.Substring($begin,$end-$begin)) 'Cut copies exact selected source text to clipboard' }
                    clear { [void](Send $editor 0x0303) }
                    delete { [void](Send $editor 0x0100 0x2e); [void](Send $editor 0x0101 0x2e) }
                    backspace { [void](Send $editor 0x0100 8); [void](Send $editor 0x0101 8) }
                    replace { [void](Send $editor 0x0102 ([int][char]'X')) }
                    enter { [void](Send $editor 0x0102 13) }
                }
                $nativeAfter=Native-Text $editor
                if($operation -eq 'enter'){
                    $after=State $main
                    Check ($after.source -ceq $before.source -and $after.dirty -eq $before.dirty -and $after.undo -eq $before.undo -and $after.redo -eq $before.redo -and $after.anchor -eq $before.anchor -and $after.active -eq $before.active) 'Standalone WM_CHAR13 is a native ignored control message and must preserve source/selection/history'
                    return [pscustomobject]@{before=$before;expected_source=$before.source;expected_history=$before.undo;after=$after;native_text_before=$nativeBefore;native_text_after=$nativeAfter;route='Standalone WM_CHAR13 control; actual Enter keydown pipeline verified separately'}
                }
                $roundtrip=RoundTrip $main $editor $before $expected ($begin+$insert.Length)
                $roundtrip|Add-Member NoteProperty native_selection_before ([pscustomobject]@{start=$nativeSelectionBefore-band0xffff;end=($nativeSelectionBefore-shr16)-band0xffff})
                $roundtrip|Add-Member NoteProperty native_text_before $nativeBefore
                $roundtrip|Add-Member NoteProperty native_text_after $nativeAfter
                if($operation -eq 'paste'){
                    $callback=[MDLiteEditNative+EnumProc]{param($window,$parameter)$name=[Text.StringBuilder]::new(64);[void][MDLiteEditNative]::GetClassName($window,$name,64);if($name.ToString()-eq'msctls_statusbar32'){$script:pasteStatusWindow=$window;return $false};return $true}
                    $script:pasteStatusWindow=[IntPtr]::Zero;[void][MDLiteEditNative]::EnumChildWindows($main,$callback,[IntPtr]::Zero)
                    $roundtrip|Add-Member NoteProperty dispatch_metadata $pasteDispatch
                    $roundtrip|Add-Member NoteProperty status_text $(if($script:pasteStatusWindow-ne[IntPtr]::Zero){Native-Text $script:pasteStatusWindow}else{'UNAVAILABLE'})
                    $roundtrip|Add-Member NoteProperty readiness_after (Send $main 0x8036)
                }
                $roundtrip
            }
        }
    }
    foreach($reverse in @($false,$true)){
        foreach($eol in @($false,$true)){
          foreach($packet in @($false,$true)){
            Run-Case "supplementary-pair-packet-$packet-eol-$eol-direction-$reverse" $body {
                param($main,$editor,$path)
                $begin=$body.IndexOf('日本語');$end=if($eol){$body.IndexOf('末尾')}else{$begin+3}
                if($reverse){Select-Source $main $end $begin}else{Select-Source $main $begin $end}
                $before=State $main
                if($packet){[void](Send $editor 0x0100 0xe7)}
                [void](Send $editor 0x0102 0xd83d)
                if($packet){[void](Send $editor 0x0101 0xe7);[void](Send $editor 0x0100 0xe7)}
                [void](Send $editor 0x0102 0xde80)
                if($packet){[void](Send $editor 0x0101 0xe7)}
                $scalar=[string][char]0xd83d+[char]0xde80
                RoundTrip $main $editor $before ($body.Substring(0,$begin)+$scalar+$body.Substring($end)) ($begin+2)
            }
          }
        }
        Run-Case "enter-key-pipeline-direction-$reverse" $body {
            param($main,$editor,$path)
            $begin=$body.IndexOf('日本語');$end=$body.IndexOf('末尾')
            if($reverse){Select-Source $main $end $begin}else{Select-Source $main $begin $end}
            $before=State $main
            [void](Send $editor 0x0100 13);[void](Send $editor 0x0102 13);[void](Send $editor 0x0101 13)
            RoundTrip $main $editor $before ($body.Substring(0,$begin)+"`r`n"+$body.Substring($end)) ($begin+2)
        }
    }
    foreach($operation in @('paste','cut')){
        Run-Case "clipboard-busy-$operation-retains-range" $body {
            param($main,$editor,$path)
            $begin=$body.IndexOf('日本語');$end=$body.IndexOf('末尾');Select-Source $main $end $begin
            $before=State $main
            if($operation -eq 'paste'){Publish-FixtureClipboard $replacement}
            if(-not[MDLiteEditNative]::OpenClipboard([IntPtr]::Zero)){throw 'Could not acquire clipboard for the bounded failure fixture.'}
            try{[void](Send $editor $(if($operation -eq 'paste'){0x0302}else{0x0300}))}finally{[void][MDLiteEditNative]::CloseClipboard()}
            $after=State $main
            Check ($after.source -ceq $before.source -and $after.dirty -eq $before.dirty -and $after.undo -eq $before.undo -and $after.redo -eq $before.redo -and $after.anchor -eq $before.anchor -and $after.active -eq $before.active -and $after.revision -eq $before.revision -and $after.saved_revision -eq $before.saved_revision) 'Clipboard open failure retains exact source, selection direction, Dirty, history and saved/current revisions'
            [pscustomobject]@{before=$before;after=$after;failure_method='Tester holds Win32 clipboard open; product OpenClipboard cannot acquire it'}
        }
    }
    $table="before`r`n`r`n| A | B | C |`r`n| :--- | :---: | ---: |`r`n| one |  | 日本語 |`r`n| three | four | five |`r`n`r`nafter`r`n"
    Run-Case 'table-enter-pipeline-reverse-selection' $table {
        param($main,$editor,$path)
        $begin=$table.IndexOf('one');Select-Source $main ($begin+3) $begin;$before=State $main
        [void](Send $editor 0x0100 13);[void](Send $editor 0x0102 13);[void](Send $editor 0x0101 13)
        RoundTrip $main $editor $before ($table.Remove($begin,3).Insert($begin,"`r`n")) ($begin+2)
    }
    Run-Case 'enter-readback-gate-preserves-before-selection' $body {
        param($main,$editor,$path)
        $begin=$body.IndexOf('日本語');$end=$body.IndexOf('末尾');Select-Source $main $end $begin;$before=State $main
        [void](Send $main 0x802e 2);[void](Send $editor 0x0100 13)
        $during=[pscustomobject]@{dirty=(Send $main 0x8045 0);undo=(Send $main 0x8045 1);revision=(Send $main 0x8045 3);saved_revision=(Send $main 0x8045 6);readiness=(Send $main 0x8036);native=Native-Text $editor;enabled=[MDLiteEditNative]::IsWindowEnabled($editor)}
        [void](Send $editor 0x0102 88);$nativeAfterRejected=Native-Text $editor
        Check ($during.dirty -eq 0 -and $during.undo -eq 0 -and $during.revision -eq $before.revision -and $during.saved_revision -eq $before.saved_revision -and -not $during.enabled) 'Injected readback failure leaves model checkpoint/history untouched and locks pending native input'
        Check ($nativeAfterRejected -ceq $during.native) 'Readback gate rejects additional typed input without discarding pending Enter'
        $edited=State $main;$expected=$body.Substring(0,$begin)+"`r`n"+$body.Substring($end)
        Check ($edited.source -ceq $expected -and $edited.undo -eq 1 -and $edited.redo -eq 0 -and $edited.dirty -and $edited.anchor -eq $begin+2) 'Readback retry commits exactly one Enter transaction and caret'
        [void](Send $editor 0x0304);$undone=State $main
        Check ($undone.source -ceq $before.source -and $undone.anchor -eq $before.anchor -and $undone.active -eq $before.active -and -not $undone.dirty) 'Undo after readback retry restores original reverse selection and source checkpoint'
        [void](Send $editor 0x0454);$redone=State $main
        Check ($redone.source -ceq $expected -and $redone.anchor -eq $edited.anchor -and $redone.active -eq $edited.active) 'Redo restores retry Enter source/caret'
        [pscustomobject]@{before=$before;during=$during;native_after_rejected_input=$nativeAfterRejected;edited=$edited;undone=$undone;redone=$redone;expected_source=$expected}
    }
    foreach ($reverse in @($false,$true)) {
        foreach ($operation in @('paste','cut','replace','clear')) {
            Run-Case "table-$operation-direction-$reverse" $table {
                param($main,$editor,$path)
                $begin=$table.IndexOf('one');$end=$begin+3
                if($reverse){Select-Source $main $end $begin}else{Select-Source $main $begin $end}
                $before=State $main
                $insert=if($operation -eq 'paste'){'東京😀e'+[char]0x301}elseif($operation -eq 'replace'){'X'}else{''}
                $expected=$table.Substring(0,$begin)+$insert+$table.Substring($end)
                switch($operation){paste{Publish-FixtureClipboard $insert;[void](Send $editor 0x0302)} cut{Cut-FixtureClipboard $editor ([uint32]$process.Id);Check ([MDLiteEditNative]::ClipboardText() -ceq 'one') 'Cut cell clipboard equals selected source'} replace{[void](Send $editor 0x0102 88)} clear{[void](Send $editor 0x0303)}}
                RoundTrip $main $editor $before $expected ($begin+$insert.Length)
            }
        }
    }
    foreach ($action in @('row-before','row-after','row-delete')) {
        Run-Case "table-$action-history" $table {
            param($main,$editor,$path)
            $begin=$table.IndexOf('one'); Select-Source $main ($begin+3) $begin
            $before=State $main
            $row="| one |  | 日本語 |`r`n"; $blank="|  |  |  |`r`n"
            $expected=switch($action){row-before{$table.Replace($row,$blank+$row)}row-after{$table.Replace($row,$row+$blank)}row-delete{$table.Replace($row,'')}}
            $command=switch($action){row-before{1057}row-after{1058}row-delete{1059}}
            [void](Send $main 0x0111 $command)
            RoundTrip $main $editor $before $expected
        }
    }
    foreach ($action in @('column-before','column-after','column-delete')) {
        Run-Case "table-$action-history" $table {
            param($main,$editor,$path)
            $begin=$table.IndexOf('one');Select-Source $main ($begin+3) $begin;$before=State $main
            $prefix="before`r`n`r`n";$suffix="`r`n`r`nafter`r`n"
            $rows=switch($action){
                column-before{"|  | A | B | C |`r`n| --- | :--- | :---: | ---: |`r`n|  | one |  | 日本語 |`r`n|  | three | four | five |"}
                column-after{"| A |  | B | C |`r`n| :--- | --- | :---: | ---: |`r`n| one |  |  | 日本語 |`r`n| three |  | four | five |"}
                column-delete{"| B | C |`r`n| :---: | ---: |`r`n|  | 日本語 |`r`n| four | five |"}
            }
            $command=switch($action){column-before{1060}column-after{1061}column-delete{1062}}
            [void](Send $main 0x0111 $command)
            RoundTrip $main $editor $before ($prefix+$rows+$suffix)
        }
    }
    Run-Case 'empty-cell-tab-navigation' $table {
        param($main,$editor,$path)
        Select-Source $main ($table.IndexOf('one'));$before=State $main
        [void](Send $editor 0x0100 9);[void](Send $editor 0x0101 9);$empty=State $main
        $cellBegin=$table.IndexOf('|  |')+1;$cellEnd=$cellBegin+2
        Check ($empty.anchor -ge $cellBegin -and $empty.anchor -le $cellEnd -and $empty.active -eq $empty.anchor) 'Tab visits the empty middle cell'
        [void](Send $editor 0x0100 9);[void](Send $editor 0x0101 9);$japanese=State $main
        Check ($japanese.anchor -eq $table.IndexOf('日本語') -and $japanese.active -eq $japanese.anchor) 'Next Tab reaches the Japanese cell'
        Check ($empty.source -ceq $before.source -and $japanese.source -ceq $before.source -and -not $empty.dirty -and -not $japanese.dirty -and $empty.undo -eq 0 -and $japanese.undo -eq 0) 'Cell navigation preserves exact source/Dirty/history'
        [pscustomobject]@{before=$before;expected_empty_range=@($cellBegin,$cellEnd);empty=$empty;expected_japanese=$table.IndexOf('日本語');japanese=$japanese}
    }
    foreach ($boundarySource in @('',"日本語😀e$([char]0x301)","日本語`r`n終端`r`n")) {
        Run-Case "document-boundary-$($boundarySource.Length)" $boundarySource {
            param($main,$editor,$path)
            $states=[Collections.Generic.List[object]]::new()
            foreach($edge in @('start','end')) {
                $offset=if($edge -eq 'start'){0}else{$boundarySource.Length};Select-Source $main $offset;$before=State $main
                $keys=if($edge -eq 'start'){@(0x25,0x26)}else{@(0x27,0x28)}
                foreach($key in $keys){[void](Send $editor 0x0100 $key);[void](Send $editor 0x0101 $key);$state=State $main;$states.Add($state);Check ($state.source -ceq $before.source -and $state.dirty -eq $before.dirty -and $state.undo -eq 0 -and $state.redo -eq 0 -and $state.anchor -eq $offset -and $state.active -eq $offset) 'Boundary arrows preserve source/history/selection without invalid offsets'}
            }
            [pscustomobject]@{states=@($states);audible_warning='NOT_RUN: no audio acquisition'}
        }
    }
    Run-Case 'save-theme-panels-redraw-history-invariants' $table {
        param($main,$editor,$path)
        $begin=$table.IndexOf('one'); Select-Source $main $begin
        [void](Send $editor 0x0102 81)
        $edited=State $main
        [void](Send $main 0x0111 1005)
        $saved=State $main
        Check ([IO.File]::ReadAllText($path) -ceq $edited.source -and -not $saved.dirty -and $saved.undo -eq $edited.undo -and $saved.anchor -eq $edited.anchor) 'Save writes exact source, clears Dirty and preserves history/caret'
        [void](Send $editor 0x0304)
        $before=State $main
        $frames=[Collections.Generic.List[object]]::new()
        foreach($theme in @(1,2)){[void](Send $main 0x802b $theme);$state=State $main;$frames.Add($state);Check ($state.source -ceq $before.source -and $state.dirty -eq $before.dirty -and $state.undo -eq $before.undo -and $state.redo -eq $before.redo -and $state.anchor -eq $before.anchor -and $state.active -eq $before.active) 'Theme preserves source/Dirty/history/directional selection'}
        foreach($command in @(1026,1026,1065,1065,1066,1066,1067)){[void](Send $main 0x0111 $command);$state=State $main;$frames.Add($state);Check ($state.source -ceq $before.source -and $state.dirty -eq $before.dirty -and $state.undo -eq $before.undo -and $state.redo -eq $before.redo -and $state.anchor -eq $before.anchor -and $state.active -eq $before.active) 'Panel hide/show/reset preserves source/Dirty/history/selection'}
        [void][MDLiteEditNative]::SetWindowPos($main,[IntPtr]::Zero,0,0,900,600,0x0004)
        [void][MDLiteEditNative]::InvalidateRect($editor,[IntPtr]::Zero,$false);[void][MDLiteEditNative]::UpdateWindow($editor)
        $resized=State $main
        Check ($resized.source -ceq $before.source -and $resized.undo -eq $before.undo -and $resized.redo -eq $before.redo -and $resized.dirty -eq $before.dirty) 'Resize/redraw preserves source/Dirty/history'
        [void](Send $editor 0x0454);$redo=State $main
        Check ($redo.source -ceq $edited.source -and -not $redo.dirty -and $redo.anchor -eq $edited.anchor -and $redo.active -eq $edited.active) 'Redo survives save/theme/panel/resize and reaches saved checkpoint'
        [pscustomobject]@{edited=$edited;saved=$saved;before_display=$before;display_states=@($frames);resized=$resized;redone=$redo}
    }
    $wrappedText=('長い日本語セル'*36)+'😀e'+[char]0x0301
    $wrappedSource="before`r`n`r`n| 名前 | 内容 |`r`n| :--- | ---: |`r`n| $wrappedText | right |`r`n|  | empty |`r`n`r`nafter`r`n"
    Run-Case 'wrapped-cell-native-hit-edit-history' $wrappedSource {
        param($main,$editor,$path)
        $cell=$wrappedSource.IndexOf($wrappedText);$target=$cell+60
        Select-Source $main $target;[void](Send $editor 0x00b7)
        $startPoint=Send $main 0x8035 $cell;$targetPoint=Send $main 0x8035 $target
        $startY=($startPoint -shr 16)-band 0xffff;$targetY=($targetPoint -shr 16)-band 0xffff
        Check ($targetY -gt $startY) 'Long Japanese cell wraps and target lies on a later visual line'
        $hit=Send $main 0x8037 0 $targetPoint
        Check ($hit -ge $target-1 -and $hit -le $target+1) 'Native caret geometry hit-tests back to the same wrapped source character'
        [void](Send $editor 0x0201 1 $targetPoint);[void](Send $editor 0x0202 0 $targetPoint)
        $before=State $main
        Check ($before.anchor -ge $cell -and $before.anchor -lt $cell+$wrappedText.Length -and $before.active -eq $before.anchor) 'Native click selects the intended wrapped cell without absorbing input'
        [void](Send $editor 0x0102 88)
        $roundtrip=RoundTrip $main $editor $before ($wrappedSource.Insert($before.active,'X')) ($before.active+1)
        $frame=Join-Path (Split-Path -Parent $path) 'wrapped-cell-after.png';Save-Frame $editor $frame
        [pscustomobject]@{cell_begin=$cell;target=$target;start_point=$startPoint;target_point=$targetPoint;source_hit=$hit;expected_hit_range=@(($target-1),($target+1));editing=$roundtrip;frame=$frame}
    }
    $sections="# A`r`nalpha`r`n## child`r`nnested`r`n# B`r`nbeta`r`n"
    Run-Case 'outline-section-native-drag-history' $sections {
        param($main,$editor,$path)
        $outline=Find-Control $main 103
        $first=Send $outline 0x110a 0 0;$second=Send $outline 0x110a 1 $first
        if($first -eq 0 -or $second -eq 0){throw 'Outline fixture roots absent.'}
        $rect=[MDLiteEditNative]::TreeRect($outline,[IntPtr]$second)
        $x=[int](($rect.Left+$rect.Right)/2);$y=[int](($rect.Top+$rect.Bottom)/2);$point=($y -shl 16)-bor($x -band 0xffff)
        [void](Send $outline 0x0201 1 $point)
        [void](Send $outline 0x0200 1 ((($y+12)-shl 16)-bor($x -band 0xffff)))
        $before=State $main
        [void](Send $outline 0x110b 8 $first)
        [void](Send $main 0x0202)
        $observed=State $main
        if($observed.source -ceq $sections -and $observed.undo -eq 0){throw 'Synthetic native tree drag did not initiate a section move; model coverage remains separate.'}
        $expected="# B`r`nbeta`r`n# A`r`nalpha`r`n## child`r`nnested`r`n"
        RoundTrip $main $editor $before $expected 0
    }
    foreach($scenario in @('move-nested-section','reject-descendant')){
        Run-Case "outline-notification-$scenario" $sections {
            param($main,$editor,$path)
            $outline=Find-Control $main 103;$first=Send $outline 0x110a 0 0;$second=Send $outline 0x110a 1 $first;$child=Send $outline 0x110a 4 $first
            if($first -eq 0 -or $second -eq 0 -or $child -eq 0){throw 'Nested outline fixture tree handles absent.'}
            Select-Source $main 9 3;$before=State $main
            $sourceItem=$first;$targetItem=if($scenario -eq 'reject-descendant'){$child}else{$second}
            $sourceOffset=0
            [uint32]$ownedPid=0;[void][MDLiteEditNative]::GetWindowThreadProcessId($main,[ref]$ownedPid)
            try{
                [MDLiteEditNative]::BeginOutlineSyntheticDrag($main,$outline,[IntPtr]$sourceItem,$sourceOffset,$ownedPid)
                [void](Send $outline 0x110b 8 $targetItem);[void](Send $main 0x0202)
                if($scenario -eq 'reject-descendant'){$after=State $main;Check ($after.source -ceq $before.source -and $after.anchor -eq $before.anchor -and $after.active -eq $before.active -and $after.undo -eq 0 -and $after.redo -eq 0 -and -not $after.dirty) 'Moving a section into its own descendant is rejected without source/selection/history changes';[pscustomobject]@{before=$before;after=$after;route='Synthetic existing TVN_BEGINDRAGW + TreeView drop highlight + product WM_LBUTTONUP; not native/physical drag'}}
                else{$expected="# B`r`nbeta`r`n# A`r`nalpha`r`n## child`r`nnested`r`n";$roundtrip=RoundTrip $main $editor $before $expected ($expected.IndexOf('# A'));$roundtrip|Add-Member NoteProperty route 'Synthetic existing notification handler; moves nested A section together, not native/physical drag';$roundtrip}
            }finally{[void](Send $main 0x001f)}
        }
    }
    Run-Case 'profile-template-crlf-cursor-history' $body {
        param($main,$editor,$path)
        $workspace=Split-Path -Parent $path;$metadata=Join-Path $workspace '.mdlite'
        [IO.Directory]::CreateDirectory((Join-Path $metadata 'templates'))|Out-Null
        $template="# 見出し`r`n本文😀`r`n{{cursor}}末尾"
        [IO.File]::WriteAllText((Join-Path $metadata 'templates/cursor.md'),$template,[Text.UTF8Encoding]::new($false))
        $profile="schema_version = 1`n[[profiles]]`nid = `"daily`"`nname = `"Fixture`"`ndirectory = `"Created`"`nfilename = `"template.md`"`ntemplate = `"templates/cursor.md`"`ncollision = `"open-existing`"`n"
        [IO.File]::WriteAllText((Join-Path $metadata 'profiles.toml'),$profile,[Text.UTF8Encoding]::new($false))
        [void](Send $main 0x0111 1009);$created=State $main
        $expected=$template.Replace('{{cursor}}','');$cursor=$template.IndexOf('{{cursor}}')
        Check ($created.source -ceq $expected -and -not $created.dirty -and $created.undo -eq 0 -and $created.redo -eq 0) 'Existing profile route creates exact saved template source with fresh document history'
        Check ($created.anchor -eq $cursor -and $created.active -eq $cursor) 'Template cursor uses UTF16 source offset through CRLF/native projection'
        $createdEditor=Find-Control $main 102;$before=$created
        [void](Send $createdEditor 0x0102 88)
        $roundtrip=RoundTrip $main $createdEditor $before ($expected.Insert($cursor,'X')) ($cursor+1)
        [pscustomobject]@{template=$template;expected_source=$expected;expected_cursor=$cursor;created=$created;editing=$roundtrip;legacy_contract='No existing template-insertion command; profile creation is a saved new document, not an Undo transaction in the old document'}
    }
    $repeatSource=(($body+"`r`n")*12)+$table
    Run-Case 'timed-arrow-caret-scroll-frames' $repeatSource {
        param($main,$editor,$path)
        Select-Source $main 0;$before=State $main
        $frames=[Collections.Generic.List[object]]::new();$samples=[Collections.Generic.List[object]]::new()
        $watch=[Diagnostics.Stopwatch]::StartNew();$nextFrame=0;$index=0
        foreach($key in @(0x27,0x28,0x25,0x26)){
            $until=$watch.ElapsedMilliseconds+1000
            do {
                [void](Send $editor 0x0100 $key);[void](Send $editor 0x0101 $key)
                [void][MDLiteEditNative]::UpdateWindow($editor)
                $caret=[MDLiteEditNative]::CaretInfo($editor)
                $samples.Add([pscustomobject]@{utc=[DateTime]::UtcNow.ToString('o');elapsed_ms=$watch.Elapsed.TotalMilliseconds;key=$key;anchor=(Send $main 0x8033);active=(Send $main 0x8034);visible_source=(Send $main 0x803e);caret_owner=$caret.Caret.ToInt64();focus=$caret.Focus.ToInt64();caret_rect=$caret.CaretRect;gui_flags=$caret.Flags})
                if($watch.ElapsedMilliseconds -ge $nextFrame){$frame=Join-Path (Split-Path -Parent $path) ('arrow-frame-'+$index+'.png');Save-Frame $editor $frame;$frames.Add([pscustomobject]@{path=$frame;utc=[DateTime]::UtcNow.ToString('o');elapsed_ms=$watch.Elapsed.TotalMilliseconds;sample_index=$samples.Count-1});$index++;$nextFrame=$watch.ElapsedMilliseconds+150}
                Start-Sleep -Milliseconds 25
            }while($watch.ElapsedMilliseconds -lt $until)
        }
        $after=State $main
        Check ($after.source -ceq $before.source -and -not $after.dirty -and $after.undo -eq 0 -and $after.redo -eq 0) 'Timed synthetic arrow repeat preserves source/Dirty/history'
        Check ($samples.Count -ge 12 -and $frames.Count -ge 8) 'Timestamped selection/scroll/caret samples and native frames cover all four timed directions'
        Check (@($samples|Where-Object {$_.caret_owner -ne $editor.ToInt64()}).Count -eq 0) 'Caret belongs to target RichEdit throughout synthetic repeat'
        [pscustomobject]@{before=$before;after=$after;samples=@($samples);frames=@($frames);duration_ms=$watch.Elapsed.TotalMilliseconds;route='Synthetic individual WM_KEYDOWN/UP repeats for one second per direction; not physical 3-5 second hold';visual_gate='Timed native frames captured; PrintWindow does not establish physical input or guaranteed visible caret pixels/100ms continuity'}
    }
    Run-Case 'independent-documents-redo-branches' $body {
        param($main,$editor,$path)
        Select-Source $main 0;[void](Send $editor 0x0102 65);[void](Send $editor 0x0304);$first=State $main
        [void](Send $main 0x0111 1001);$secondEditor=Find-Control $main 102
        [void](Send $secondEditor 0x0102 66);[void](Send $secondEditor 0x0304);[void](Send $secondEditor 0x0102 67);$second=State $main
        Check ($second.source -ceq 'C' -and $second.undo -eq 1 -and $second.redo -eq 0) 'Second document new input discards only its own Redo branch'
        $tabs=Find-Control $main 101;$center=Send $main 0x803a 0
        [void](Send $tabs 0x0201 1 $center);[void](Send $tabs 0x0202 0 $center)
        $firstAgain=State $main
        Check ($firstAgain.source -ceq $first.source -and $firstAgain.undo -eq $first.undo -and $firstAgain.redo -eq 1 -and $firstAgain.anchor -eq $first.anchor) 'Returning to first document retains its independent Redo and caret'
        $firstEditor=Find-Control $main 102;[void](Send $firstEditor 0x0454);$firstRedo=State $main
        Check ($firstRedo.source -ceq ('A'+$body) -and $firstRedo.undo -eq 1 -and $firstRedo.redo -eq 0 -and $firstRedo.anchor -eq 1) 'First document Redo restores only its own transaction'
        [pscustomobject]@{first_undone=$first;second_branch=$second;first_restored=$firstAgain;first_redone=$firstRedo}
    }
    $endSourceHash=Get-SourceHash
    $sourceStable=$sourceHash -eq $endSourceHash
    if($cases.Count -eq 0){throw "No cases matched CaseFilter: $CaseFilter"}
    $result=[pscustomobject]@{schema='mdlite-prehuman-edit-v1';preset=$Preset;commit=(& git -C $repoRoot rev-parse HEAD);build_receipt=$receipt;executable_path=$executable;executable_sha256=$exeHash;source_sha256=$sourceHash;source_sha256_after=$endSourceHash;source_stable=$sourceStable;cases=@($cases);failed=@($cases|Where-Object status -eq 'FAIL'|ForEach-Object name);blocked=@($cases|Where-Object status -eq 'BLOCKED'|ForEach-Object name);all_automated_cases_pass=$sourceStable -and @($cases|Where-Object status -ne 'PASS').Count -eq 0;human_lane='NOT_RUN: physical keyboard/mouse/drag and real Microsoft IME/ATOK composition';timestamp_utc=[DateTime]::UtcNow.ToString('o')}
    $result | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
    Write-Output "Evidence: $OutputPath; PASS=$(@($cases|Where-Object status -eq 'PASS').Count), FAIL=$($result.failed.Count), BLOCKED=$($result.blocked.Count), source_stable=$sourceStable"
    if(-not $result.all_automated_cases_pass){exit 1}
} finally {
    try {
        $cleanup='UNCHANGED'
        if($clipboardTrackingUnconfirmed){$cleanup='UNCONFIRMED_CUT_CLIPBOARD_GENERATION_EXTERNAL_DATA_PROTECTED'}
        elseif($clipboardChanged){
            if([MDLiteEditNative]::GetClipboardSequenceNumber() -ne $lastTestClipboardSequence){$cleanup='SKIPPED_NEWER_CLIPBOARD_PRESERVED'}
            else{if($null -ne $clipboard){[Windows.Forms.Clipboard]::SetDataObject($clipboard,$true)}else{[Windows.Forms.Clipboard]::Clear()};$cleanup='RESTORED_FROM_EAGER_MEMORY_SNAPSHOT'}
        }
        elseif($clipboardExternalGeneration){$cleanup='SKIPPED_EXTERNAL_GENERATION_PRESERVED'}
    } catch {$cleanup='BLOCKED_RESTORE: '+$_.Exception.GetType().Name}
    finally {
        [MDLiteEditNative]::CloseFixtureClipboardOwner()
        if(Test-Path -LiteralPath $OutputPath){$finalEvidence=Get-Content -Raw -LiteralPath $OutputPath|ConvertFrom-Json;$finalEvidence|Add-Member NoteProperty clipboard_cleanup $cleanup -Force;if($cleanup -like 'BLOCKED*' -or $cleanup -like 'UNCONFIRMED*'){$finalEvidence.all_automated_cases_pass=$false};$finalEvidence|ConvertTo-Json -Depth 12|Set-Content -LiteralPath $OutputPath -Encoding utf8}
        if($null -eq $oldSilent){Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue}else{$env:MDLITE_TEST_SILENT=$oldSilent}
        if($null -eq $oldCrash){Remove-Item Env:MDLITE_TEST_NO_CRASH_UI -ErrorAction SilentlyContinue}else{$env:MDLITE_TEST_NO_CRASH_UI=$oldCrash}
    }
}
if($cleanup -like 'BLOCKED*' -or $cleanup -like 'UNCONFIRMED*'){exit 1}
