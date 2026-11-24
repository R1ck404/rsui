package dev.rsui;

import android.app.Activity;
import android.app.AlertDialog;
import android.app.DatePickerDialog;
import android.app.Dialog;
import android.app.TimePickerDialog;
import android.content.ActivityNotFoundException;
import android.content.Context;
import android.content.DialogInterface;
import android.content.Intent;
import android.content.res.Configuration;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.ColorFilter;
import android.graphics.Insets;
import android.graphics.Paint;
import android.graphics.PixelFormat;
import android.graphics.Rect;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.Drawable;
import android.os.Build;
import android.os.Bundle;
import android.text.InputType;
import android.text.format.DateFormat;
import android.view.ContextThemeWrapper;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import android.widget.Button;
import android.widget.DatePicker;
import android.widget.TimePicker;
import android.widget.Toast;

import java.util.Calendar;
import java.util.Locale;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * The activity an rsui app runs in. It has one view, a SurfaceView, whose surface rsui draws and which hears what comes in:
 * touches, keys, and the soft keyboard's text through an InputConnection, which Android's own NativeActivity has no way to
 * give. Everything is passed to the native library as it comes, and the app's
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
        // rsui draws into a surface of its own that's kept clear of the status bar, the navigation bar, the camera's cutout and
        // the soft keyboard, by padding what holds it by the insets the system gives. So where a finger is on the view is where
        // it is in what's drawn, and the field being typed in is never under the keyboard.
        Window window = getWindow();
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false);
        } else {
            window.getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
        // The view takes text, so Android would bring the keyboard up as the app opens; it comes only for a field rsui has
        // focused (showKeyboard).
        window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE
                | WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN);
        window.setStatusBarColor(Color.TRANSPARENT);
        window.setNavigationBarColor(Color.TRANSPARENT);
        view = new RsuiView(this);
        view.getHolder().addCallback(this);
        FrameLayout holder = new FrameLayout(this);
        holder.addView(view, new FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT));
        holder.setOnApplyWindowInsetsListener(new View.OnApplyWindowInsetsListener() {
            @Override
            public WindowInsets onApplyWindowInsets(View holder, WindowInsets insets) {
                if (Build.VERSION.SDK_INT >= 30) {
                    Insets clear = insets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout()
                            | WindowInsets.Type.ime());
                    holder.setPadding(clear.left, clear.top, clear.right, clear.bottom);
                } else {
                    holder.setPadding(insets.getSystemWindowInsetLeft(), insets.getSystemWindowInsetTop(),
                            insets.getSystemWindowInsetRight(), insets.getSystemWindowInsetBottom());
                }
                return insets;
            }
        });
        holder.setBackground(bars);
        if (Build.VERSION.SDK_INT >= 29) {
            // The bars are the app's own colors, as they're given, without a scrim the system would put over them.
            window.setStatusBarContrastEnforced(false);
            window.setNavigationBarContrastEnforced(false);
        }
        setContentView(holder);
        barColors(getResources().getConfiguration());
        view.requestFocus();
        nativeCreate(this, getAssets(), getFilesDir().getPath(), getCacheDir().getPath(),
                getResources().getDisplayMetrics().density, isNight(getResources().getConfiguration()));
    }

    private static boolean isNight(Configuration configuration) {
        return (configuration.uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
    }

    /** What's behind the status bar and the navigation bar: the colors at the top and bottom of what rsui draws. */
    private final BarsBehind bars = new BarsBehind();
    /** Those colors, as rsui last said them, as ARGB, or 0 where what it draws there isn't one solid color. */
    private int appTop, appBottom;

    /**
     * Called by the app's thread when the colors at the top and bottom edges of what's drawn change, so the bars beside them
     * are the same, and the app goes on under them, as a phone's own apps do.
     */
    public void barColors(final int top, final int bottom) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                appTop = top;
                appBottom = bottom;
                barColors(getResources().getConfiguration());
            }
        });
    }

    /**
     * Has the bars the app's colors, or where it has none, light or dark as the phone is, with their icons dark on a light
     * color and light on a dark one.
     */
    private void barColors(Configuration configuration) {
        boolean night = isNight(configuration);
        int otherwise = night ? Color.BLACK : Color.WHITE;
        int top = appTop != 0 ? appTop : otherwise;
        int bottom = appBottom != 0 ? appBottom : otherwise;
        bars.set(top, bottom);
        Window window = getWindow();
        window.setBackgroundDrawable(new ColorDrawable(otherwise));
        boolean lightTop = isLight(top);
        boolean lightBottom = isLight(bottom);
        if (Build.VERSION.SDK_INT >= 30) {
            WindowInsetsController controller = window.getInsetsController();
            if (controller != null) {
                int wanted = (lightTop ? WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS : 0)
                        | (lightBottom ? WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS : 0);
                controller.setSystemBarsAppearance(wanted,
                        WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS);
            }
        } else {
            View decor = window.getDecorView();
            int flags = decor.getSystemUiVisibility() & ~(View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR);
            flags |= (lightTop ? View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR : 0) | (lightBottom ? View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR : 0);
            decor.setSystemUiVisibility(flags);
        }
    }

    /** Whether a color is light enough that dark icons show on it better than light ones. */
    private static boolean isLight(int color) {
        double luminance = (0.2126 * Color.red(color) + 0.7152 * Color.green(color) + 0.0722 * Color.blue(color)) / 255.0;
        return luminance > 0.6;
    }

    /**
     * Paints the strip under the status bar in the top color, and everything else in the bottom one: the navigation bar, and
     * the keyboard's room while it comes and goes. Where rsui's surface is, it's what shows.
     */
    final class BarsBehind extends Drawable {
        private final Paint paint = new Paint();
        private int top = Color.BLACK, bottom = Color.BLACK;

        void set(int top, int bottom) {
            if (top != this.top || bottom != this.bottom) {
                this.top = top;
                this.bottom = bottom;
                invalidateSelf();
            }
        }

        @Override
        public void draw(Canvas canvas) {
            Rect bounds = getBounds();
            View holder = view == null ? null : (View) view.getParent();
            int strip = holder == null ? 0 : holder.getPaddingTop();
            paint.setColor(bottom);
            canvas.drawRect(bounds, paint);
            paint.setColor(top);
            canvas.drawRect(bounds.left, bounds.top, bounds.right, bounds.top + strip, paint);
        }

        @Override
        public void setAlpha(int alpha) {}

        @Override
        public void setColorFilter(ColorFilter filter) {}

        @Override
        public int getOpacity() {
            return PixelFormat.OPAQUE;
        }
    }

    @Override
    public void onConfigurationChanged(Configuration configuration) {
        super.onConfigurationChanged(configuration);
        barColors(configuration);
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

    /**
     * Called by the app's thread to show one of the system's popups, which is answered through nativePopupAnswered with
     * the same id: "true" or "false" for a confirm, a place in the list for a sheet, "2026-11-21" for a day, "14:05" for a
     * time, anything for an alert or share, and null for none; or "!" and why it couldn't be shown.
     */
    public void popup(final int id, final String kind, final String json) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                Answer answer = new Answer(id);
                try {
                    show(answer, kind, new JSONObject(json));
                } catch (JSONException | RuntimeException failed) {
                    answer.say("!" + failed);
                }
            }
        });
    }

    /** Called by the app's thread to show a toast. */
    public void toast(final String text, final boolean lasting) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                Toast.makeText(RsuiActivity.this, text, lasting ? Toast.LENGTH_LONG : Toast.LENGTH_SHORT).show();
            }
        });
    }

    /** A popup's answer, which is only said once, whichever way it's answered. */
    static class Answer {
        private final int id;
        private boolean said;

        Answer(int id) {
            this.id = id;
        }

        void say(String answer) {
            if (!said) {
                said = true;
                nativePopupAnswered(id, answer);
            }
        }

        /** Says this once the dialog's gone, however it went, unless something's been said already. */
        void otherwise(Dialog dialog, final String answer) {
            dialog.setOnDismissListener(new DialogInterface.OnDismissListener() {
                @Override
                public void onDismiss(DialogInterface gone) {
                    say(answer);
                }
            });
        }
    }

    /** The activity with the system's own light or dark look, as it is now, for the popups. */
    private Context themed() {
        int theme = isNight(getResources().getConfiguration())
                ? android.R.style.Theme_DeviceDefault
                : android.R.style.Theme_DeviceDefault_Light;
        return new ContextThemeWrapper(this, theme);
    }

    private static int[] day(String text) {
        String[] parts = text.split("-");
        return new int[] { Integer.parseInt(parts[0]), Integer.parseInt(parts[1]), Integer.parseInt(parts[2]) };
    }

    private static long dayMillis(String text) {
        int[] day = day(text);
        Calendar calendar = Calendar.getInstance();
        calendar.clear();
        calendar.set(day[0], day[1] - 1, day[2]);
        return calendar.getTimeInMillis();
    }

    private static AlertDialog.Builder titled(Context context, String title, String message) {
        AlertDialog.Builder builder = new AlertDialog.Builder(context);
        if (!title.isEmpty()) {
            builder.setTitle(title);
        }
        if (!message.isEmpty()) {
            builder.setMessage(message);
        }
        return builder;
    }

    private void show(final Answer answer, String kind, JSONObject asked) throws JSONException {
        Context context = themed();
        String title = asked.optString("title", "");
        String message = asked.optString("message", "");
        switch (kind) {
            case "confirm": {
                AlertDialog.Builder builder = titled(context, title, message);
                builder.setPositiveButton(asked.optString("confirm", getString(android.R.string.ok)), new DialogInterface.OnClickListener() {
                    @Override
                    public void onClick(DialogInterface dialog, int which) {
                        answer.say("true");
                    }
                });
                builder.setNegativeButton(asked.optString("cancel", getString(android.R.string.cancel)), null);
                AlertDialog dialog = builder.create();
                answer.otherwise(dialog, "false");
                dialog.show();
                if (asked.optBoolean("destructive", false)) {
                    // Material's color for what can't be undone.
                    Button confirm = dialog.getButton(DialogInterface.BUTTON_POSITIVE);
                    if (confirm != null) {
                        boolean night = isNight(getResources().getConfiguration());
                        confirm.setTextColor(night ? Color.rgb(0xF2, 0xB8, 0xB5) : Color.rgb(0xB3, 0x26, 0x1E));
                    }
                }
                break;
            }
            case "alert": {
                AlertDialog.Builder builder = titled(context, title, message);
                builder.setPositiveButton(asked.optString("ok", getString(android.R.string.ok)), null);
                AlertDialog dialog = builder.create();
                answer.otherwise(dialog, "");
                dialog.show();
                break;
            }
            case "sheet": {
                JSONArray given = asked.getJSONArray("options");
                String[] options = new String[given.length()];
                for (int at = 0; at < options.length; at++) {
                    options[at] = given.getString(at);
                }
                AlertDialog.Builder builder = titled(context, title, "");
                builder.setItems(options, new DialogInterface.OnClickListener() {
                    @Override
                    public void onClick(DialogInterface dialog, int which) {
                        answer.say(Integer.toString(which));
                    }
                });
                builder.setNegativeButton(asked.optString("cancel", getString(android.R.string.cancel)), null);
                AlertDialog dialog = builder.create();
                answer.otherwise(dialog, null);
                dialog.show();
                break;
            }
            case "date": {
                Calendar now = Calendar.getInstance();
                int[] start = asked.has("value") ? day(asked.getString("value"))
                        : new int[] { now.get(Calendar.YEAR), now.get(Calendar.MONTH) + 1, now.get(Calendar.DAY_OF_MONTH) };
                DatePickerDialog dialog = new DatePickerDialog(context, new DatePickerDialog.OnDateSetListener() {
                    @Override
                    public void onDateSet(DatePicker picker, int year, int month, int day) {
                        answer.say(String.format(Locale.ROOT, "%04d-%02d-%02d", year, month + 1, day));
                    }
                }, start[0], start[1] - 1, start[2]);
                if (asked.has("min")) {
                    dialog.getDatePicker().setMinDate(dayMillis(asked.getString("min")));
                }
                if (asked.has("max")) {
                    dialog.getDatePicker().setMaxDate(dayMillis(asked.getString("max")));
                }
                answer.otherwise(dialog, null);
                dialog.show();
                break;
            }
            case "time": {
                Calendar now = Calendar.getInstance();
                int hours = now.get(Calendar.HOUR_OF_DAY);
                int minutes = now.get(Calendar.MINUTE);
                if (asked.has("value")) {
                    String[] parts = asked.getString("value").split(":");
                    hours = Integer.parseInt(parts[0]);
                    minutes = Integer.parseInt(parts[1]);
                }
                TimePickerDialog dialog = new TimePickerDialog(context, new TimePickerDialog.OnTimeSetListener() {
                    @Override
                    public void onTimeSet(TimePicker picker, int hours, int minutes) {
                        answer.say(String.format(Locale.ROOT, "%02d:%02d", hours, minutes));
                    }
                }, hours, minutes, DateFormat.is24HourFormat(this));
                answer.otherwise(dialog, null);
                dialog.show();
                break;
            }
            case "share": {
                String text = asked.optString("text", "");
                String url = asked.optString("url", "");
                String said = text.isEmpty() ? url : url.isEmpty() ? text : text + "\n" + url;
                Intent send = new Intent(Intent.ACTION_SEND);
                send.setType("text/plain");
                send.putExtra(Intent.EXTRA_TEXT, said);
                if (!title.isEmpty()) {
                    send.putExtra(Intent.EXTRA_SUBJECT, title);
                    send.putExtra(Intent.EXTRA_TITLE, title);
                }
                try {
                    startActivity(Intent.createChooser(send, title.isEmpty() ? null : title));
                    answer.say("");
                } catch (ActivityNotFoundException failed) {
                    answer.say("!nothing on this phone can share that");
                }
                break;
            }
            default:
                answer.say("!there's no popup called " + kind);
        }
    }

    /** The surface rsui draws, which input comes to, so a touch is where it is in what's drawn. */
    static class RsuiView extends SurfaceView {
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
    static native void nativePopupAnswered(int id, String answer);
}
