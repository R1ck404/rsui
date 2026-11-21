package dev.rsui;

import android.app.Activity;
import android.content.Context;
import android.content.res.Configuration;
import android.os.Bundle;
import android.text.InputType;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.View;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

/**
 * The activity an rsui app runs in. It hands its window's surface to rsui, which draws it, and keeps one view of its own,
 * which is never drawn, for what comes in: touches, keys, and the soft keyboard's text through an InputConnection, which
 * Android's own NativeActivity has no way to give. Everything is passed to the native library as it comes, and the app's
 * own thread there does the rest.
 */
public class RsuiActivity extends Activity implements SurfaceHolder.Callback2 {
    static {
        System.loadLibrary("rsui_app");
    }

    private RsuiView view;

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        getWindow().takeSurface(this);
        view = new RsuiView(this);
        setContentView(view);
        view.requestFocus();
        nativeCreate(this, getAssets(), getFilesDir().getPath(), getCacheDir().getPath(),
                getResources().getDisplayMetrics().density, isNight(getResources().getConfiguration()));
    }

    private static boolean isNight(Configuration configuration) {
        return (configuration.uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
    }

    @Override
    public void onConfigurationChanged(Configuration configuration) {
        super.onConfigurationChanged(configuration);
        nativeConfiguration(getResources().getDisplayMetrics().density, isNight(configuration));
    }

    @Override
    protected void onResume() {
        super.onResume();
        nativeShown(true);
    }

    @Override
    protected void onPause() {
        nativeShown(false);
        super.onPause();
    }

    @Override
    public void onWindowFocusChanged(boolean focused) {
        super.onWindowFocusChanged(focused);
        nativeFocus(focused);
    }

    @Override
    public void onLowMemory() {
        super.onLowMemory();
        nativeLowMemory();
    }

    @Override
    protected void onDestroy() {
        nativeDestroy();
        super.onDestroy();
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        nativeSurface(holder.getSurface());
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        nativeSurface(holder.getSurface());
    }

    @Override
    public void surfaceRedrawNeeded(SurfaceHolder holder) {
        nativeRedraw();
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        nativeSurface(null);
    }

    /** Called by the app's thread when a field that takes text is focused, or isn't any more. */
    public void showKeyboard(final boolean show) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                InputMethodManager keyboard = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
                if (show) {
                    view.requestFocus();
                    keyboard.restartInput(view);
                    keyboard.showSoftInput(view, 0);
                } else {
                    keyboard.hideSoftInputFromWindow(view.getWindowToken(), 0);
                }
            }
        });
    }

    /** The view that's never drawn, which input comes to. */
    static class RsuiView extends View {
        RsuiView(Context context) {
            super(context);
            setFocusable(true);
            setFocusableInTouchMode(true);
        }

        @Override
        public boolean onTouchEvent(MotionEvent event) {
            int action = event.getActionMasked();
            // A move is of every finger that's down; anything else is of the one it says.
            if (action == MotionEvent.ACTION_MOVE) {
                for (int at = 0; at < event.getPointerCount(); at++) {
                    nativeTouch(action, event.getPointerId(at), event.getX(at), event.getY(at));
                }
            } else {
                int index = event.getActionIndex();
                nativeTouch(action, event.getPointerId(index), event.getX(index), event.getY(index));
            }
            return true;
        }

        @Override
        public boolean onKeyDown(int code, KeyEvent event) {
            if (code == KeyEvent.KEYCODE_BACK) {
                return super.onKeyDown(code, event);
            }
            nativeKey(code, event.getMetaState(), true, event.getRepeatCount(), event.getUnicodeChar());
            return true;
        }

        @Override
        public boolean onKeyUp(int code, KeyEvent event) {
            if (code == KeyEvent.KEYCODE_BACK) {
                return super.onKeyUp(code, event);
            }
            nativeKey(code, event.getMetaState(), false, 0, 0);
            return true;
        }

        @Override
        public boolean onCheckIsTextEditor() {
            return true;
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo info) {
            info.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_AUTO_CORRECT;
            info.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN | EditorInfo.IME_FLAG_NO_EXTRACT_UI;
            return new RsuiInput(this);
        }
    }

    /** What the soft keyboard says, as rsui's own composing text and typing. */
    static class RsuiInput extends BaseInputConnection {
        RsuiInput(View view) {
            super(view, false);
        }

        @Override
        public boolean setComposingText(CharSequence text, int cursor) {
            nativeCompose(text.toString());
            return true;
        }

        @Override
        public boolean finishComposingText() {
            nativeFinishComposing();
            return true;
        }

        @Override
        public boolean commitText(CharSequence text, int cursor) {
            nativeCommit(text.toString());
            return true;
        }

        @Override
        public boolean deleteSurroundingText(int before, int after) {
            nativeDelete(before, after);
            return true;
        }

        @Override
        public boolean sendKeyEvent(KeyEvent event) {
            boolean down = event.getAction() == KeyEvent.ACTION_DOWN;
            nativeKey(event.getKeyCode(), event.getMetaState(), down, event.getRepeatCount(), down ? event.getUnicodeChar() : 0);
            return true;
        }
    }

    static native void nativeCreate(RsuiActivity activity, Object assets, String files, String cache, float density, boolean night);
    static native void nativeConfiguration(float density, boolean night);
    static native void nativeShown(boolean shown);
    static native void nativeFocus(boolean focused);
    static native void nativeLowMemory();
    static native void nativeDestroy();
    static native void nativeSurface(Object surface);
    static native void nativeRedraw();
    static native void nativeTouch(int action, int pointer, float x, float y);
    static native void nativeKey(int code, int meta, boolean down, int repeat, int unicode);
    static native void nativeCompose(String text);
    static native void nativeFinishComposing();
    static native void nativeCommit(String text);
    static native void nativeDelete(int before, int after);
}
