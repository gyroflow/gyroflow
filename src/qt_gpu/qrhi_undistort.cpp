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

// ubufAlignment
// static inline uint aligned(uint v, uint byteAlign) { return (v + byteAlign - 1) & ~(byteAlign - 1); }

class QtRHIUndistort {
public:
    QtRHIUndistort(MDKPlayerWrapper *playerWrapper): m_player(playerWrapper) { }

    bool init(MDKPlayer *item, QSize textureSize, QSizeF /*itemSize*/, QSize outputSize, const QString &shaderPath, int kernelParmsSize) {
        if (!item) return false;
        auto context = item->rhiContext();
        auto rhi = context->rhi();

        m_outputSize = outputSize;

        m_initialUpdates = rhi->nextResourceUpdateBatch();

        m_texIn.reset(rhi->newTexture(QRhiTexture::RGBA8, textureSize, 1, QRhiTexture::UsedAsTransferSource));
        if (!m_texIn->create()) { qDebug() << "failed to create m_texIn"; return false; }

        m_workaroundTexture.reset(rhi->newTexture(QRhiTexture::RGBA8, QSize(16, 16), 1, QRhiTexture::UsedAsTransferSource));
        if (!m_workaroundTexture->create()) { qDebug() << "failed to create m_workaroundTexture"; return false; }

        m_kernelParams.reset(rhi->newBuffer(QRhiBuffer::Dynamic, QRhiBuffer::UniformBuffer, kernelParmsSize));
        if (!m_kernelParams->create()) { qDebug() << "failed to create m_kernelParams"; return false; }

        m_texMatrices.reset(rhi->newTexture(QRhiTexture::R32F, QSize(9, textureSize.height()), 1, QRhiTexture::UsedAsTransferSource));
        if (!m_texMatrices->create()) { qDebug() << "failed to create m_texMatrices"; return false; }

        matrices.resize(textureSize.height() * 9);
        kernel_params.resize(kernelParmsSize);

        m_vertexBuffer.reset(rhi->newBuffer(QRhiBuffer::Immutable, QRhiBuffer::VertexBuffer, sizeof(quadVertexData)));
        if (!m_vertexBuffer->create()) { qDebug() << "failed to create m_vertexBuffer"; return false; }
        m_initialUpdates->uploadStaticBuffer(m_vertexBuffer.get(), quadVertexData);

        m_indexBuffer.reset(rhi->newBuffer(QRhiBuffer::Immutable, QRhiBuffer::IndexBuffer, sizeof(quadIndexData)));
        if (!m_indexBuffer->create()) { qDebug() << "failed to create m_indexBuffer"; return false; }
        m_initialUpdates->uploadStaticBuffer(m_indexBuffer.get(), quadIndexData);

        m_drawingUniform.reset(rhi->newBuffer(QRhiBuffer::Dynamic, QRhiBuffer::UniformBuffer, 64 + 4));
        if (!m_drawingUniform->create()) { qDebug() << "failed to create m_drawingUniform"; return false; }
        qint32 flip = rhi->isYUpInFramebuffer();
        m_initialUpdates->updateDynamicBuffer(m_drawingUniform.get(), 64, 4, &flip);

        m_drawingSampler.reset(rhi->newSampler(QRhiSampler::Linear, QRhiSampler::Linear, QRhiSampler::None, QRhiSampler::ClampToEdge, QRhiSampler::ClampToEdge));
        if (!m_drawingSampler->create()) { qDebug() << "failed to create m_drawingSampler"; return false; }

        m_matricesSampler.reset(rhi->newSampler(QRhiSampler::Nearest, QRhiSampler::Nearest, QRhiSampler::None, QRhiSampler::ClampToEdge, QRhiSampler::ClampToEdge));
        if (!m_matricesSampler->create()) { qDebug() << "failed to create m_matricesSampler"; return false; }

        m_srb.reset(rhi->newShaderResourceBindings());
        m_srb->setBindings({
            QRhiShaderResourceBinding::uniformBuffer (0, QRhiShaderResourceBinding::FragmentStage | QRhiShaderResourceBinding::VertexStage, m_drawingUniform.get()),
            QRhiShaderResourceBinding::sampledTexture(1, QRhiShaderResourceBinding::FragmentStage, m_texIn.get(), m_drawingSampler.get()),
            QRhiShaderResourceBinding::uniformBuffer (2, QRhiShaderResourceBinding::FragmentStage, m_kernelParams.get()),
            QRhiShaderResourceBinding::sampledTexture(3, QRhiShaderResourceBinding::FragmentStage, m_texMatrices.get(), m_matricesSampler.get()),
        });
        if (!m_srb->create()) { qDebug() << "failed to create m_srb"; return false; }

        m_pipeline.reset(rhi->newGraphicsPipeline());
        m_pipeline->setShaderStages({
            { QRhiShaderStage::Vertex,   getShader(QLatin1String(":/src/qt_gpu/compiled/texture.vert.qsb")) },
            { QRhiShaderStage::Fragment, getShader(shaderPath) }
        });
        QRhiVertexInputLayout inputLayout;
        inputLayout.setBindings({ { 4 * sizeof(float) } });
        inputLayout.setAttributes({
            { 0, 0, QRhiVertexInputAttribute::Float2, 0 },
            { 0, 1, QRhiVertexInputAttribute::Float2, 2 * sizeof(float) }
        });
        m_pipeline->setVertexInputLayout(inputLayout);
        m_pipeline->setShaderResourceBindings(m_srb.get());
        m_pipeline->setRenderPassDescriptor(item->rhiRenderPassDescriptor());
        if (!m_pipeline->create()) { qDebug() << "failed to create m_pipeline"; return false; }

        return true;
    }

    bool render(MDKPlayer *item) {
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

        if (item->qmlWindow()->rendererInterface()->graphicsApi() == QSGRendererInterface::Direct3D11Rhi) {
            // Workaround for the synchronization issue
            // Reading a dummy texture causes the outstanding draw operations to flush
            if (!m_readbackResult) m_readbackResult.reset(new QRhiReadbackResult());
            u->readBackTexture({ m_workaroundTexture.get() }, m_readbackResult.get());
        }

        u->copyTexture(m_texIn.get(), item->rhiTexture(), {});

        u->updateDynamicBuffer(m_kernelParams.get(), 0, kernel_params.size(), kernel_params.data());

        QRhiTextureSubresourceUploadDescription desc1(matrices.data(), matrices.size() * sizeof(float));

        u->uploadTexture(m_texMatrices.get(), QRhiTextureUploadDescription({ QRhiTextureUploadEntry(0, 0, desc1) }));

        QMatrix4x4 mvp = item->textureMatrix();
        mvp.scale(2.0f);
        u->updateDynamicBuffer(m_drawingUniform.get(), 0, 64, mvp.constData());

        cb->resourceUpdate(u);
        u = rhi->nextResourceUpdateBatch();

        cb->beginPass(item->rhiRenderTarget(), QColor(Qt::black), { 1.0f, 0 }, u);
        cb->setGraphicsPipeline(m_pipeline.get());
        cb->setViewport({ 0, 0, float(size.width()), float(size.height()) });
        cb->setShaderResources();
        QRhiCommandBuffer::VertexInput vbufBinding(m_vertexBuffer.get(), 0);
        cb->setVertexInput(0, 1, &vbufBinding, m_indexBuffer.get(), 0, QRhiCommandBuffer::IndexUInt16);
        cb->drawIndexed(6);
        cb->endPass();

        return true;
    }

    std::vector<unsigned char> kernel_params;
    std::vector<float> matrices;

    QShader getShader(const QString &name) {
        QFile f(name);
        if (f.open(QIODevice::ReadOnly))
            return QShader::fromSerialized(f.readAll());
        return QShader();
    }

    QScopedPointer<QRhiTexture> m_texIn;
    QScopedPointer<QRhiTexture> m_workaroundTexture;
    QScopedPointer<QRhiTexture> m_texMatrices;
    QScopedPointer<QRhiBuffer> m_kernelParams;

    QSize m_outputSize;

    MDKPlayerWrapper *m_player{nullptr};

    QScopedPointer<QRhiBuffer> m_vertexBuffer;
    QScopedPointer<QRhiBuffer> m_indexBuffer;
    QScopedPointer<QRhiBuffer> m_drawingUniform;
    QScopedPointer<QRhiSampler> m_drawingSampler;
    QScopedPointer<QRhiSampler> m_matricesSampler;
    QScopedPointer<QRhiShaderResourceBindings> m_srb;
    QScopedPointer<QRhiGraphicsPipeline> m_pipeline;

    QScopedPointer<QRhiReadbackResult> m_readbackResult;

    QRhiResourceUpdateBatch *m_initialUpdates{nullptr};
};
