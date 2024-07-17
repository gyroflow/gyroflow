package xyz.gyroflow;

import android.os.*;
import android.content.*;
import android.net.Uri;
//import android.util.Log;

public class MainActivity extends org.qtproject.qt.android.bindings.QtActivity {
    public static native void urlReceived(String url);

    @Override
    public void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        Intent intent = getIntent();
        if (intent != null && intent.getAction() != null) {
            processIntent(intent);
        }
    }

    @Override
    public void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        processIntent(intent);
    }

    private void processIntent(Intent intent) {
        Uri uri;
        if ("android.intent.action.VIEW".equals(intent.getAction())) {
            uri = intent.getData();
        } else if ("android.intent.action.SEND".equals(intent.getAction())) {
            uri = (Uri)intent.getExtras().get(Intent.EXTRA_STREAM);
        } else {
            return;
        }
        if (uri != null) {
            urlReceived(uri.toString());
        }
    }
}
