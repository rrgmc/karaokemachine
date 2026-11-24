// The karaoke machine on a Samsung Tizen television: a NUI application that hands its window and
// its folders to the machine in lib/libkm_app.so, and is otherwise empty.
//
// The machine runs on this application's UI thread and does not return until it ends: each frame
// it lets Ecore do its work (docs/plans/tizen-port.md, the nested loop). It withholds Back from
// DALi, and when it stops it ends this application with ui_app_exit. So once km_tizen_run is
// called, nothing here runs again except the log callback.

using System;
using System.Reflection;
using System.Runtime.InteropServices;
using Tizen;
using Tizen.Applications;
using Tizen.NUI;
using Tizen.NUI.BaseComponents;

namespace KaraokeMachine
{
    class Program : NUIApplication
    {
        const string LogTag = "KaraokeMachine";

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        delegate void LogFn(IntPtr line);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        delegate int RunFn(IntPtr ecoreWindow, int width, int height, IntPtr dataDir, IntPtr assetDir, LogFn log);

        // Kept in a static so the delegate the machine calls back into is never collected.
        static readonly LogFn NativeLog = p => Log.Info(LogTag, Marshal.PtrToStringUTF8(p) ?? "");

        // Both are kept in fields so the delegate DALi calls and the timer are never collected.
        Window.FrameCallbackType firstFrame;
        Timer fallback;
        bool started;

        protected override void OnCreate()
        {
            base.OnCreate();
            Window.Instance.BackgroundColor = Color.Black;
            // Only once a frame of this window has reached the screen. The television keeps its launch
            // animation up until then, and the machine, once called, keeps the thread: posted straight
            // from here, DALi never drew, and the set showed its spinner over the home screen for ever
            // while the machine ran unseen behind it. The timer is for a set that never reports.
            firstFrame = _ => CoreApplication.Post(StartOnce);
            Window.Instance.AddFramePresentedCallback(firstFrame, 0);
            fallback = new Timer(3000);
            fallback.Tick += (s, e) => { StartOnce(); return false; };
            fallback.Start();
        }

        void StartOnce()
        {
            if (started) return;
            started = true;
            fallback.Stop();
            StartMachine();
        }

        void StartMachine()
        {
            string failure;
            try
            {
                Tizen.Applications.DirectoryInfo dirs = Application.Current.DirectoryInfo;
                string library = System.IO.Path.Combine(dirs.Resource, "..", "lib", "libkm_app.so");
                string assets = System.IO.Path.Combine(dirs.Resource, "assets");
                IntPtr lib = NativeLibrary.Load(System.IO.Path.GetFullPath(library));
                var run = Marshal.GetDelegateForFunctionPointer<RunFn>(NativeLibrary.GetExport(lib, "km_tizen_run"));
                Size2D size = Window.Instance.WindowSize;
                IntPtr data = Marshal.StringToCoTaskMemUTF8(dirs.Data);
                IntPtr res = Marshal.StringToCoTaskMemUTF8(assets);
                try
                {
                    Log.Info(LogTag, "starting the machine: " + size.Width + "x" + size.Height + ", data " + dirs.Data);
                    int code = run(NativeWindow(), size.Width, size.Height, data, res, NativeLog);
                    Log.Info(LogTag, "the machine returned " + code);
                    if (code == 0)
                        return;
                    failure = "The karaoke machine stopped with an error (" + code + ").";
                }
                finally
                {
                    Marshal.FreeCoTaskMem(data);
                    Marshal.FreeCoTaskMem(res);
                }
            }
            catch (Exception e)
            {
                Log.Error(LogTag, "could not start the machine: " + e);
                failure = "The karaoke machine could not start: " + e.Message;
            }
            // Said on the screen as well as in the log, since nobody reads a television's log.
            // Back ends the application from here: the machine is no longer withholding it.
            Window.Instance.KeyEvent += (s, e) =>
            {
                if (e.Key.State == Key.StateType.Down && e.Key.KeyPressedName == "XF86Back")
                    Exit();
            };
            Window.Instance.Add(new TextLabel(failure)
            {
                WidthResizePolicy = ResizePolicyType.FillToParent,
                HeightResizePolicy = ResizePolicyType.FillToParent,
                MultiLine = true,
                PointSize = 24f,
                TextColor = Color.White,
                Padding = new Extents(120, 120, 120, 120),
            });
        }

        // The Ecore_Wl2_Window* behind the NUI window, from NUI's own getter. It is internal in every
        // TizenFX from API 8 on, so it is called by reflection, as OmegaTube does. The public route,
        // Tizen.Common.IWindowProvider.WindowHandle, arrived after API 10 and returns the same thing.
        static IntPtr NativeWindow()
        {
            MethodInfo getter = typeof(Window).GetMethod("GetNativeWindowHandler",
                BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public);
            if (getter == null) throw new InvalidOperationException("NUI's Window has no GetNativeWindowHandler");
            IntPtr handle = (IntPtr)getter.Invoke(Window.Instance, null);
            if (handle == IntPtr.Zero) throw new InvalidOperationException("NUI's Window has no native window");
            return handle;
        }

        static void Main(string[] args)
        {
            new Program().Run(args);
        }
    }
}
