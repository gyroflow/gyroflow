// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

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
    QRhiTexture *rhiTexture2();
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

static float quadVertexData[16] = { // Y up, CCW
    -0.5f,  0.5f, 0.0f, 0.0f,
    -0.5f, -0.5f, 0.0f, 1.0f,
    0.5f, -0.5f, 1.0f, 1.0f,
    0.5f,  0.5f, 1.0f, 0.0f
};
static quint16 quadIndexData[6] = { 0, 1, 2, 0, 2, 3 };


struct Uniforms {
    quint32 params_count;
    quint32 width;
    quint32 height;
    quint32 output_width;
    quint32 output_height;
    quint32 _padding;
    quint32 _padding2;
    quint32 _padding3;
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

        m_outputSize = outputSize;

        m_initialUpdates = rhi->nextResourceUpdateBatch();

        m_texIn = rhi->newTexture(QRhiTexture::RGBA8, textureSize, 1, QRhiTexture::UsedAsTransferSource);
        m_texIn->create();
        m_releasePool << m_texIn;

        //m_texOut = rhi->newTexture(QRhiTexture::RGBA8, m_outputSize, 1, QRhiTexture::UsedWithLoadStore);
        //m_texOut->create();
        //m_releasePool << m_texOut;

        m_computeUniform = rhi->newBuffer(QRhiBuffer::Dynamic, QRhiBuffer::UniformBuffer, sizeof(Uniforms));
        m_computeUniform->create();
        m_releasePool << m_computeUniform;

        m_texParams = rhi->newTexture(QRhiTexture::R32F, QSize(9, (textureSize.height() + 1)), 1, QRhiTexture::UsedAsTransferSource);
        m_texParams->create();
        m_releasePool << m_texParams;
        params_buffer.resize((textureSize.height() + 1) * 9);

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

        qint32 flip = rhi->isYUpInFramebuffer();
        m_initialUpdates->updateDynamicBuffer(m_drawingUniform, 64, 4, &flip);

        m_drawingSampler = rhi->newSampler(QRhiSampler::Linear, QRhiSampler::Linear, QRhiSampler::None, QRhiSampler::ClampToEdge, QRhiSampler::ClampToEdge);
        m_releasePool << m_drawingSampler;
        m_drawingSampler->create();

        m_paramsSampler = rhi->newSampler(QRhiSampler::Nearest, QRhiSampler::Nearest, QRhiSampler::None, QRhiSampler::ClampToEdge, QRhiSampler::ClampToEdge);
        m_releasePool << m_paramsSampler;
        m_paramsSampler->create();

        m_srb = rhi->newShaderResourceBindings();
        m_releasePool << m_srb;
        m_srb->setBindings({
            QRhiShaderResourceBinding::uniformBuffer(0, QRhiShaderResourceBinding::VertexStage | QRhiShaderResourceBinding::FragmentStage, m_drawingUniform),
            QRhiShaderResourceBinding::sampledTexture(1, QRhiShaderResourceBinding::FragmentStage, m_texIn, m_drawingSampler),
            QRhiShaderResourceBinding::uniformBuffer(2, QRhiShaderResourceBinding::FragmentStage, m_computeUniform),
            QRhiShaderResourceBinding::sampledTexture(3, QRhiShaderResourceBinding::FragmentStage, m_texParams, m_paramsSampler),
        });
        m_srb->create();


        m_pipeline = rhi->newGraphicsPipeline();
        m_releasePool << m_pipeline;
        m_pipeline->setShaderStages({
            { QRhiShaderStage::Vertex,   getShader(QLatin1String(":/src/qt_gpu/compiled/texture.vert.qsb")) },
            { QRhiShaderStage::Fragment, getShader(QLatin1String(":/src/qt_gpu/compiled/undistort.frag.qsb")) } 
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

        return true;
    }

    void cleanup() {
        qDeleteAll(m_releasePool);
        m_releasePool.clear();
    }

    bool render(MDKPlayer *item, double /*timestamp*/, int /*frame_no*/, float */*params_padded*/, int params_count, float bg[4], bool /*doRender*/, float */*features_pixels*/, int /*fpx_count*/, float */*optflow_pixels*/, int /*of_count*/) {
        if (!item->qmlItem() || !item->rhiTexture() || !item->qmlWindow()) return false;
        auto context = item->rhiContext();
        auto rhi = context->rhi();

        const QSize size = item->textureSize();
        QRhiCommandBuffer *cb = context->currentFrameCommandBuffer();

        if (item->qmlWindow()->rendererInterface()->graphicsApi() == QSGRendererInterface::Direct3D11Rhi) {
            // Workaround for the synchronization issue
            // For some reason reading a dummy texture causes the pipeline to flush or something
            auto *u = rhi->nextResourceUpdateBatch();
            QRhiReadbackResult *rbResult = new QRhiReadbackResult();
            rbResult->completed = [rbResult] { delete rbResult; };
            u->readBackTexture({ item->rhiTexture2() }, rbResult);
            cb->resourceUpdate(u);
        }

        QRhiResourceUpdateBatch *u = rhi->nextResourceUpdateBatch();
        if (m_initialUpdates) {
            u->merge(m_initialUpdates);
            m_initialUpdates->release();
            m_initialUpdates = nullptr;
        }

        u->copyTexture(m_texIn, item->rhiTexture(), {});

        Uniforms uniforms;
        uniforms.params_count = params_count - 1;
        uniforms.width = size.width();
        uniforms.height = size.height();
        uniforms.output_width = m_outputSize.width();
        uniforms.output_height = m_outputSize.height();
        memcpy(uniforms.bg, bg, 4 * sizeof(float)); // RGBA
        u->updateDynamicBuffer(m_computeUniform, 0, sizeof(Uniforms), (const char *)&uniforms);

        QRhiTextureSubresourceUploadDescription desc1(params_buffer.data(), params_buffer.size() * sizeof(float));

        u->uploadTexture(m_texParams, QRhiTextureUploadDescription({ QRhiTextureUploadEntry(0, 0, desc1) }));

        QMatrix4x4 mvp = item->textureMatrix();
        mvp.scale(2.0f);
        u->updateDynamicBuffer(m_drawingUniform, 0, 64, mvp.constData());

        cb->resourceUpdate(u);
        u = rhi->nextResourceUpdateBatch();

        QColor clearColor(Qt::black);
        cb->beginPass(item->rhiRenderTarget(), clearColor, { 1.0f, 0 });
        cb->setGraphicsPipeline(m_pipeline);
        cb->setViewport({ 0, 0, float(size.width()), float(size.height()) });
        cb->setShaderResources();
        QRhiCommandBuffer::VertexInput vbufBinding(m_vertexBuffer, 0);
        cb->setVertexInput(0, 1, &vbufBinding, m_indexBuffer, 0, QRhiCommandBuffer::IndexUInt16);
        cb->drawIndexed(6);
        cb->endPass(u);

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

    QRhiTexture *m_texIn = nullptr;
    // QRhiTexture *m_texOut = nullptr;
    QRhiTexture *m_texParams = nullptr;
    QRhiBuffer *m_computeUniform = nullptr;

    QSize m_outputSize;

    MDKPlayerWrapper *m_player{nullptr};

    QRhiBuffer *m_vertexBuffer = nullptr;
    QRhiBuffer *m_indexBuffer = nullptr;
    QRhiBuffer *m_drawingUniform = nullptr;
    QRhiSampler *m_drawingSampler = nullptr;
    QRhiSampler *m_paramsSampler = nullptr;
    QRhiShaderResourceBindings *m_srb = nullptr;
    QRhiGraphicsPipeline *m_pipeline = nullptr;

    QRhiResourceUpdateBatch *m_initialUpdates = nullptr;
};
