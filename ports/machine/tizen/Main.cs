// The karaoke machine on a Samsung Tizen television: a NUI application that hands its window and
// its folders to the machine in lib/libkm_app.so, and is otherwise empty.
//
// The machine runs on this application's UI thread and does not return until it ends: each frame
// it lets Ecore do its work (docs/architecture/tizen.md, the nested loop). It withholds Back from
// DALi, and when it stops it ends this application with ui_app_exit. So once km_tizen_run is
// called, nothing here runs again except the log callback and OnPause and OnResume, which the
// Ecore iterate inside the machine's loop dispatches.

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

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        delegate void SetVisibleFn(int visible);

        // Kept in a static so the delegate the machine calls back into is never collected.
        static readonly LogFn NativeLog = p => Log.Info(LogTag, Marshal.PtrToStringUTF8(p) ?? "");

        // Kept in fields so the delegate DALi calls and the timers are never collected.
        Window.FrameCallbackType firstFrame;
        Timer settle, fallback;
        TextLabel starting;
        bool started;
        // The machine's km_tizen_set_visible, once the library is loaded, and whether the app is
        // paused now, so a pause that came before the library is passed on when it loads.
        SetVisibleFn setVisible;
        bool paused;

        // Tizen pauses the app when none of its windows can be seen: Home, another input, an app
        // over it. The machine then stops drawing and pauses a playing song, as on Android. These
        // run on the UI thread, from inside the Ecore iterate the machine's frame loop turns, so they
        // only store a flag in the machine and return. See docs/architecture/tizen.md, Hidden.
        protected override void OnPause()
        {
            base.OnPause();
            paused = true;
            Log.Info(LogTag, "paused");
            setVisible?.Invoke(0);
        }

        protected override void OnResume()
        {
            base.OnResume();
            paused = false;
            Log.Info(LogTag, "resumed");
            setVisible?.Invoke(1);
        }

        protected override void OnCreate()
        {
            base.OnCreate();
            Window.Instance.BackgroundColor = Color.Black;
            // The window needs something in it to be drawn, and drawn before the machine takes the
            // thread: the television keeps its launch animation up until the application's window is
            // on screen, and the machine's picture is a subsurface of that window. With an empty
            // window, and with the machine started on the first presented frame, the set showed its
            // spinner over the home screen for ever while the machine ran unseen. The probe app, whose
            // window held a label and which started the machine long after, never did.
            starting = new TextLabel("Karaoke Machine")
            {
                WidthResizePolicy = ResizePolicyType.FillToParent,
                HeightResizePolicy = ResizePolicyType.FillToParent,
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                PointSize = 24f,
                TextColor = Color.White,
            };
            Window.Instance.Add(starting);
            // A second after the first frame has been presented, or five seconds after creation if the
            // set never reports one.
            settle = new Timer(1000);
            settle.Tick += (s, e) => { StartOnce(); return false; };
            firstFrame = _ => CoreApplication.Post(() => { if (!started) settle.Start(); });
            Window.Instance.AddFramePresentedCallback(firstFrame, 0);
            fallback = new Timer(5000);
            fallback.Tick += (s, e) => { StartOnce(); return false; };
            fallback.Start();
        }

        void StartOnce()
        {
            if (started) return;
            started = true;
            settle.Stop();
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
                setVisible = Marshal.GetDelegateForFunctionPointer<SetVisibleFn>(NativeLibrary.GetExport(lib, "km_tizen_set_visible"));
                if (paused)
                    setVisible(0);
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
            Window.Instance.Remove(starting);
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
