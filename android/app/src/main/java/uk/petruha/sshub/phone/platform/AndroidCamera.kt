package uk.petruha.sshub.phone.platform

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import android.util.Size
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import uk.petruha.sshub.phone.core.CameraPermission
import uk.petruha.sshub.phone.core.CameraPort
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import androidx.camera.core.Preview as CameraPreview

/**
 * CameraX luma frames for the scan screens. Frames go straight to the callback and are never stored. The Activity calls `attach` in
 * `onCreate` (before STARTED) because the permission launcher must be registered then.
 */
class AndroidCamera(private val context: Context) : CameraPort {
    private val _permission = MutableStateFlow(CameraPermission.Unknown)
    override val permission: StateFlow<CameraPermission> = _permission.asStateFlow()
    private var activity: ComponentActivity? = null
    private var launcher: ActivityResultLauncher<String>? = null
    private var asked = false

    override val hasCamera: Boolean get() = context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)

    fun attach(activity: ComponentActivity) {
        this.activity = activity
        launcher = activity.registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            asked = true
            refresh(granted)
        }
        activity.lifecycle.addObserver(LifecycleEventObserver { _, e -> if (e == Lifecycle.Event.ON_RESUME) refresh() })
    }

    /** Re-reads the system state (the user may have changed it in Settings). */
    fun refresh(grantedNow: Boolean = isGranted()) {
        val a = activity
        _permission.value = when {
            grantedNow -> CameraPermission.Granted
            !asked || a == null -> CameraPermission.Unknown
            ActivityCompat.shouldShowRequestPermissionRationale(a, Manifest.permission.CAMERA) -> CameraPermission.Denied
            else -> CameraPermission.DeniedForever
        }
    }

    private fun isGranted() = ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED

    override fun requestPermission() {
        if (isGranted()) { refresh(true); return }
        launcher?.launch(Manifest.permission.CAMERA)
    }

    override fun openAppSettings() {
        val i = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
        (activity ?: context).let { c -> if (c !is android.app.Activity) i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK); c.startActivity(i) }
    }

    @Composable
    override fun Preview(modifier: Modifier, onLuma: (width: Int, height: Int, luma: ByteArray) -> Unit) {
        val ctx = LocalContext.current
        val owner = LocalLifecycleOwner.current
        val sink = rememberUpdatedState(onLuma)
        val view = remember { PreviewView(ctx).apply { scaleType = PreviewView.ScaleType.FILL_CENTER } }
        DisposableEffect(owner, view) {
            var running: Binding? = null
            val observer = LifecycleEventObserver { _, e ->
                when (e) {
                    Lifecycle.Event.ON_RESUME -> if (running == null && isGranted()) {
                        running = bind(ctx, owner, view) { w, h, l -> sink.value(w, h, l) }
                    }
                    Lifecycle.Event.ON_PAUSE -> { running?.stop(); running = null }
                    else -> {}
                }
            }
            owner.lifecycle.addObserver(observer)
            onDispose {
                owner.lifecycle.removeObserver(observer)
                running?.stop()
            }
        }
        AndroidView(factory = { view }, modifier = modifier)
    }

    private class Binding {
        @Volatile var stopped = false
        var provider: ProcessCameraProvider? = null
        var analysis: ImageAnalysis? = null
        val executor: ExecutorService = Executors.newSingleThreadExecutor()
        fun stop() {
            stopped = true
            analysis?.clearAnalyzer()
            provider?.unbindAll()
            executor.shutdown()
        }
    }

    private fun bind(ctx: Context, owner: LifecycleOwner, view: PreviewView, onLuma: (Int, Int, ByteArray) -> Unit): Binding {
        val b = Binding()
        val future = ProcessCameraProvider.getInstance(ctx)
        future.addListener({
            if (b.stopped) return@addListener
            try {
                val provider = future.get()
                val preview = CameraPreview.Builder().build().also { it.surfaceProvider = view.surfaceProvider }
                val analysis = ImageAnalysis.Builder()
                    .setResolutionSelector(
                        ResolutionSelector.Builder()
                            .setResolutionStrategy(ResolutionStrategy(Size(1280, 720), ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER))
                            .build(),
                    )
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
                analysis.setAnalyzer(b.executor) { image ->
                    try {
                        val w = image.width
                        val h = image.height
                        val plane = image.planes[0]
                        val buf = plane.buffer
                        val stride = plane.rowStride
                        val out = ByteArray(w * h)
                        if (stride == w) {
                            buf.rewind()
                            buf.get(out, 0, w * h)
                        } else {
                            for (y in 0 until h) {
                                buf.position(y * stride)
                                buf.get(out, y * w, w)
                            }
                        }
                        onLuma(w, h, out)
                    } finally {
                        image.close()
                    }
                }
                val selector = if (provider.hasCamera(CameraSelector.DEFAULT_BACK_CAMERA)) CameraSelector.DEFAULT_BACK_CAMERA else CameraSelector.DEFAULT_FRONT_CAMERA
                provider.unbindAll()
                provider.bindToLifecycle(owner, selector, preview, analysis)
                b.provider = provider
                b.analysis = analysis
            } catch (_: Exception) {
                // no usable camera: the scan screen keeps showing the text alternative
            }
        }, ContextCompat.getMainExecutor(ctx))
        return b
    }
}
