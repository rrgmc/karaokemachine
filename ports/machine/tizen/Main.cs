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

        [DllImport("libdali2-csharp-binder.so", EntryPoint = "CSharp_Dali_GetNativeWindowHandler")]
        static extern IntPtr DaliNativeWindow(HandleRef window);

        protected override void OnCreate()
        {
            base.OnCreate();
            Window.Instance.BackgroundColor = Color.Black;
            // After OnCreate has returned, so the window is shown before the machine takes the thread.
            CoreApplication.Post(StartMachine);
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

        // The Ecore_Wl2_Window* behind the NUI window. NUI keeps the getter internal, so its handle
        // is read by reflection and handed to DALi's own C binding.
        static IntPtr NativeWindow()
        {
            const BindingFlags any = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public;
            object cptr = null;
            for (Type t = typeof(Window); t != null && cptr == null; t = t.BaseType)
            {
                PropertyInfo prop = t.GetProperty("SwigCPtr", any);
                if (prop != null) cptr = prop.GetValue(Window.Instance);
                FieldInfo field = t.GetField("swigCPtr", any);
                if (cptr == null && field != null) cptr = field.GetValue(Window.Instance);
            }
            if (cptr == null) throw new InvalidOperationException("NUI's Window has no SwigCPtr");
            return DaliNativeWindow((HandleRef)cptr);
        }

        static void Main(string[] args)
        {
            new Program().Run(args);
        }
    }
}
