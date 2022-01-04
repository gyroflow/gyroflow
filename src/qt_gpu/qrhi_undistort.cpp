
#include <QQuickWindow>
#include <QFile>
#include <private/qquickitem_p.h>
#include <private/qrhi_p.h>
#include <private/qsgrenderer_p.h>
#include <private/qsgdefaultrendercontext_p.h>
#include <private/qshader_p.h>

class MDKPlayer {
public:
    QSGDefaultRenderContext *rhiContext();
    QRhiTexture *rhiTexture();
    QRhiTextureRenderTarget *rhiRenderTarget();
    QRhiRenderPassDescriptor *rhiRenderPassDescriptor();
    QQuickWindow *qmlWindow();
    QQuickItem *qmlItem();
    QSize textureSize();
    QMatrix4x4 textureMatrix();

    void setupGpuCompute(std::function<bool(QSize texSize, QSizeF itemSize)> &&initCb, std::function<bool(double, int32_t, bool)> &&renderCb, std::function<void()> &&cleanupCb);
    void cleanupGpuCompute();
};
class MDKPlayerWrapper {
public:
    MDKPlayer *mdkplayer;
};

#define DRAW_TO_RENDERTARGET

#ifdef DRAW_TO_RENDERTARGET
    static float quadVertexData[16] = { // Y up, CCW
        -0.5f,  0.5f, 0.0f, 0.0f,
        -0.5f, -0.5f, 0.0f, 1.0f,
        0.5f, -0.5f, 1.0f, 1.0f,
        0.5f,  0.5f, 1.0f, 0.0f
    };
    static quint16 quadIndexData[6] = { 0, 1, 2, 0, 2, 3 };
#endif

struct Uniforms {
    quint32 params_count;
    quint32 width;
    quint32 height;
    quint32 _padding;
    float bg[4];
};

// ubufAlignment
// static inline uint aligned(uint v, uint byteAlign) { return (v + byteAlign - 1) & ~(byteAlign - 1); }

class QtRHIUndistort {
public:
    bool init(MDKPlayer *item, QSize textureSize, QSizeF /*itemSize*/, QSize outputSize) {
        if (!item) return false;
        auto context = item->rhiContext();
        auto rhi = context->rhi();

        if (!rhi->isFeatureSupported(QRhi::Compute)) {
            qWarning("Compute is not supported");
            return false;
        }

        m_outputSize = outputSize;

        m_initialUpdates = rhi->nextResourceUpdateBatch();

        // -------- Compute pass init --------

        // m_texIn = rhi->newTexture(QRhiTexture::RGBA8, textureSize, 1, QRhiTexture::UsedWithLoadStore | QRhiTexture::UsedAsTransferSource);
        // m_texIn->create();
        // m_releasePool << m_texIn;

        m_texOut = rhi->newTexture(QRhiTexture::RGBA8, m_outputSize, 1, QRhiTexture::UsedWithLoadStore);
        m_texOut->create();
        m_releasePool << m_texOut;

        m_computeUniform = rhi->newBuffer(QRhiBuffer::Dynamic, QRhiBuffer::UniformBuffer, sizeof(Uniforms));
        m_computeUniform->create();
        m_releasePool << m_computeUniform;

        m_computeParams = rhi->newBuffer(QRhiBuffer::Immutable, QRhiBuffer::StorageBuffer | QRhiBuffer::VertexBuffer, (textureSize.height() + 1) * 12 * sizeof(float));
        m_computeParams->create();
        m_releasePool << m_computeParams;

        m_featuresPixels = rhi->newBuffer(QRhiBuffer::Immutable, QRhiBuffer::StorageBuffer | QRhiBuffer::VertexBuffer, (1 * sizeof(float)));
        m_featuresPixels->create();
        m_releasePool << m_featuresPixels;

        m_optflowPixels = rhi->newBuffer(QRhiBuffer::Immutable, QRhiBuffer::StorageBuffer | QRhiBuffer::VertexBuffer, (1 * sizeof(float)));
        m_optflowPixels->create();
        m_releasePool << m_optflowPixels;

        m_computeBindings = rhi->newShaderResourceBindings();
        m_computeBindings->setBindings({
            QRhiShaderResourceBinding::imageLoad(0, QRhiShaderResourceBinding::ComputeStage, item->rhiTexture(), 0),
            QRhiShaderResourceBinding::imageStore(1, QRhiShaderResourceBinding::ComputeStage, m_texOut, 0),
            QRhiShaderResourceBinding::uniformBuffer(2, QRhiShaderResourceBinding::ComputeStage, m_computeUniform),
            QRhiShaderResourceBinding::bufferLoad(3, QRhiShaderResourceBinding::ComputeStage, m_computeParams),
            QRhiShaderResourceBinding::bufferLoad(4, QRhiShaderResourceBinding::ComputeStage, m_featuresPixels),
            QRhiShaderResourceBinding::bufferLoad(5, QRhiShaderResourceBinding::ComputeStage, m_optflowPixels)
        });
        m_computeBindings->create();
        m_releasePool << m_computeBindings;

        m_computePipeline = rhi->newComputePipeline();
        m_computePipeline->setShaderResourceBindings(m_computeBindings);
        m_computePipeline->setShaderStage({ QRhiShaderStage::Compute, getShader(QLatin1String(":/src/qt_gpu/compiled/undistort.comp.qsb")) });
        m_computePipeline->create();
        m_releasePool << m_computePipeline;

        // -------- Compute pass init --------

#ifdef DRAW_TO_RENDERTARGET
        // -------- Graphics pass init --------
        m_vertexBuffer = rhi->newBuffer(QRhiBuffer::Immutable, QRhiBuffer::VertexBuffer, sizeof(quadVertexData));
        m_vertexBuffer->create();
        m_releasePool << m_vertexBuffer;

        m_initialUpdates->uploadStaticBuffer(m_vertexBuffer, quadVertexData);

        m_indexBuffer = rhi->newBuffer(QRhiBuffer::Immutable, QRhiBuffer::IndexBuffer, sizeof(quadIndexData));
        m_indexBuffer->create();
        m_releasePool << m_indexBuffer;

        m_initialUpdates->uploadStaticBuffer(m_indexBuffer, quadIndexData);

        m_drawingUniform = rhi->newBuffer(QRhiBuffer::Dynamic, QRhiBuffer::UniformBuffer, 64 + 4);
        m_drawingUniform->create();
        m_releasePool << m_drawingUniform;

        qint32 flip = 0; // regardless of isYUpInFramebuffer() since the input is not flipped so the end result is good for GL too
        m_initialUpdates->updateDynamicBuffer(m_drawingUniform, 64, 4, &flip);

        m_drawingSampler = rhi->newSampler(QRhiSampler::Linear, QRhiSampler::Linear, QRhiSampler::None, QRhiSampler::ClampToEdge, QRhiSampler::ClampToEdge);
        m_releasePool << m_drawingSampler;
        m_drawingSampler->create();

        m_srb = rhi->newShaderResourceBindings();
        m_releasePool << m_srb;
        m_srb->setBindings({
            QRhiShaderResourceBinding::uniformBuffer(0, QRhiShaderResourceBinding::VertexStage | QRhiShaderResourceBinding::FragmentStage, m_drawingUniform),
            QRhiShaderResourceBinding::sampledTexture(1, QRhiShaderResourceBinding::FragmentStage, m_texOut, m_drawingSampler)
        });
        m_srb->create();

        // -------- Graphics pass init --------

        m_pipeline = rhi->newGraphicsPipeline();
        m_releasePool << m_pipeline;
        m_pipeline->setShaderStages({
            { QRhiShaderStage::Vertex,   getShader(QLatin1String(":/src/qt_gpu/compiled/texture.vert.qsb")) },
            { QRhiShaderStage::Fragment, getShader(QLatin1String(":/src/qt_gpu/compiled/texture.frag.qsb")) }
        });
        QRhiVertexInputLayout inputLayout;
        inputLayout.setBindings({ { 4 * sizeof(float) } });
        inputLayout.setAttributes({
            { 0, 0, QRhiVertexInputAttribute::Float2, 0 },
            { 0, 1, QRhiVertexInputAttribute::Float2, 2 * sizeof(float) }
        });
        m_pipeline->setVertexInputLayout(inputLayout);
        m_pipeline->setShaderResourceBindings(m_srb);
        m_pipeline->setRenderPassDescriptor(item->rhiRenderPassDescriptor());
        m_pipeline->create();
#endif
        return true;
    }

    void cleanup() {
        qDeleteAll(m_releasePool);
        m_releasePool.clear();
    }

    bool render(MDKPlayer *item, double /*timestamp*/, int /*frame_no*/, const float *params_padded, int params_count, float bg[4], bool /*doRender*/, float *features_pixels, int fpx_count, float *optflow_pixels, int of_count) {
        if (!item->qmlItem() || !item->rhiTexture() || !item->qmlWindow()) return false;
        auto context = item->rhiContext();
        auto rhi = context->rhi();

        const QSize size = item->textureSize();
        QRhiCommandBuffer *cb = context->currentFrameCommandBuffer();

        QRhiResourceUpdateBatch *u = rhi->nextResourceUpdateBatch();
        if (m_initialUpdates) {
            u->merge(m_initialUpdates);
            m_initialUpdates->release();
            m_initialUpdates = nullptr;
        }

/*#ifndef DRAW_TO_RENDERTARGET
        u->copyTexture(m_texIn, item->rhiTexture(), {});
#endif*/

        Uniforms uniforms;
        uniforms.params_count = params_count - 1;
        uniforms.width = size.width();
        uniforms.height = size.height();
        memcpy(uniforms.bg, bg, 4 * sizeof(float)); // RGBA
        u->updateDynamicBuffer(m_computeUniform, 0, sizeof(Uniforms), (const char *)&uniforms);

        u->uploadStaticBuffer(m_computeParams, 0, params_count * 12 * sizeof(float), params_padded);

        if (features_pixels && fpx_count) {
            m_featuresPixels->setSize(fpx_count * sizeof(float));
            m_featuresPixels->create();
            u->uploadStaticBuffer(m_featuresPixels, features_pixels);
        }
        if (optflow_pixels && of_count) {
            m_optflowPixels->setSize(of_count * sizeof(float));
            m_optflowPixels->create();
            u->uploadStaticBuffer(m_optflowPixels, optflow_pixels);
        }

#ifdef DRAW_TO_RENDERTARGET
        QMatrix4x4 mvp = item->textureMatrix();
        mvp.scale(2.0f);
        u->updateDynamicBuffer(m_drawingUniform, 0, 64, mvp.constData());
#endif

        cb->beginComputePass(u);
        cb->setComputePipeline(m_computePipeline);
        cb->setShaderResources();
        cb->dispatch(m_outputSize.width() / 16, m_outputSize.height() / 16, 1);
        cb->endComputePass();

#ifndef DRAW_TO_RENDERTARGET
        u = rhi->nextResourceUpdateBatch();
        QRhiTextureCopyDescription desc;
        desc.setPixelSize(size);
        u->copyTexture(item->rhiTexture(), m_texOut, desc);
        cb->resourceUpdate(u);
#endif

#ifdef DRAW_TO_RENDERTARGET
        QColor clearColor(Qt::black);
        cb->beginPass(item->rhiRenderTarget(), clearColor, { 1.0f, 0 });
        cb->setGraphicsPipeline(m_pipeline);
        cb->setViewport({ 0, 0, float(size.width()), float(size.height()) });
        cb->setShaderResources();
        QRhiCommandBuffer::VertexInput vbufBinding(m_vertexBuffer, 0);
        cb->setVertexInput(0, 1, &vbufBinding, m_indexBuffer, 0, QRhiCommandBuffer::IndexUInt16);
        cb->drawIndexed(6);
        cb->endPass();
#endif
        return true;
    }

    std::vector<float> params_buffer;

    QShader getShader(const QString &name) {
        QFile f(name);
        if (f.open(QIODevice::ReadOnly))
            return QShader::fromSerialized(f.readAll());
        return QShader();
    }

    QList<QRhiResource *> m_releasePool;

    // QRhiTexture *m_texIn = nullptr;
    QRhiTexture *m_texOut = nullptr;
    QRhiBuffer *m_computeUniform = nullptr;
    QRhiBuffer *m_computeParams = nullptr;
    QRhiBuffer *m_featuresPixels = nullptr;
    QRhiBuffer *m_optflowPixels = nullptr;
    QRhiShaderResourceBindings *m_computeBindings = nullptr;
    QRhiComputePipeline *m_computePipeline = nullptr;

    QSize m_outputSize;

    MDKPlayerWrapper *m_player{nullptr};

#ifdef DRAW_TO_RENDERTARGET
    QRhiBuffer *m_vertexBuffer = nullptr;
    QRhiBuffer *m_indexBuffer = nullptr;
    QRhiBuffer *m_drawingUniform = nullptr;
    QRhiSampler *m_drawingSampler = nullptr;
    QRhiShaderResourceBindings *m_srb = nullptr;
    QRhiGraphicsPipeline *m_pipeline = nullptr;
#endif

    QRhiResourceUpdateBatch *m_initialUpdates = nullptr;

};
