// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 yasumorishima <fwyasu11@gmail.com>
//
// gmflow-scale2-regrefine6 (Xu et al., "Unifying Flow, Stereo and Depth Estimation", TPAMI 2023) written by hand in burn,
// ported from unimatch (https://github.com/autonomousvision/unimatch, at AdrianEddy/unimatch da140fa), whose code is
// under this license:
//
//   MIT License
//
//   Copyright (c) 2022 autonomousvision
//
//   Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
//   documentation files (the "Software"), to deal in the Software without restriction, including without limitation the
//   rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit
//   persons to whom the Software is furnished to do so, subject to the following conditions:
//
//   The above copyright notice and this permission notice shall be included in all copies or substantial portions of the
//   Software.
//
//   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE
//   WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
//   COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//   OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//
// Only what the flow task runs at inference with the settings Gyroflow uses: 2 scales (1/8 then 1/4), split-window
// attention with 2 and 8 splits, global matching at 1/8 and local (radius 4) at 1/4, global then local (radius 1)
// propagation, 6 refinement iterations, convex upsampling. Module and parameter names follow unimatch, so the
// tensors of the checkpoint map onto them by name (see the weights conversion).

use burn::module::{ Module, Param };
use burn::nn::conv::{ Conv2d, Conv2dConfig };
use burn::nn::{ InstanceNorm, InstanceNormConfig, LayerNorm, LayerNormConfig, Linear, LinearConfig, PaddingConfig2d };
use burn::tensor::activation::{ gelu, relu, sigmoid, softmax, tanh };
use burn::tensor::module::conv2d;
use burn::tensor::ops::{ ConvOptions, GridSampleOptions, GridSamplePaddingMode, InterpolateMode, PadMode };
use burn::tensor::{ Device, Tensor, TensorData };

use super::{ coop, kernels };

pub const FEATURE_CHANNELS: usize = 128;
pub const REFINE_ITERATIONS: usize = 6;
const LOCAL_RADIUS: usize = 4; // matching at 1/4 and the correlation of the refinement
const UPSAMPLE: usize = 4;
/// The largest buffer the im2col convolutions make: Vulkan guarantees 128 MiB for one storage buffer
const IM2COL_MAX_BYTES: usize = 120 << 20;

/// Which implementation runs the operations that have more than one: the CubeCL kernels of kernels.rs or the plain tensor
/// operations they replace (the reference), and convolutions as im2col + matmul or as burn convolutions
#[derive(Clone, Copy, Debug)]
pub struct Ops {
    pub kernels: bool,
    pub conv_im2col: bool,
    /// The GRU of the refinement with its convolutions split by input (GruFactored); needs the im2col kernel
    pub gru_factored: bool,
    /// The matmuls of the im2col convolutions and of the factored GRU by kernels::gemm instead of burn's (for devices
    /// without f32 matrix units)
    pub own_gemm: bool,
    /// The transformer's linear layers by kernels::gemm too
    pub gemm_linear: bool,
    /// Split-window attention by kernels::window_attention (no window layout or roll in memory, the softmax in one
    /// launch), the transformer's LayerNorms with their residual adds by kernels::layer_norm, the CNN's InstanceNorms with
    /// their relus and residual adds by kernels::instance_norm, the factored GRU's gates by kernels::gru_gate and the flow
    /// head's last convolution (2 outputs) by kernels::conv_small
    pub fused: bool,
    /// The im2col convolutions with own_gemm (Co >= 32) and the GRU's by kernels::conv_gemm: the columns gathered in the
    /// matmul instead of made in memory
    pub implicit: bool,
    /// Every matmul and convolution of the model (the 1x1 ones included; not the attention's, nor the flow head's last
    /// with `fused`) on the matrix units through WGSL cooperative matrices (coop.rs), before all the flags above: for
    /// devices whose kernels are WGSL and which offer them (Apple GPUs through Metal)
    pub coop: bool,
}

/// Which matmul runs a matmul of the model, and the convolutions built on one
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gemm {
    Burn,
    /// kernels::gemm (and kernels::conv_gemm for the gathered columns)
    Own,
    /// coop::gemm and coop::conv_gemm
    Coop,
}

impl Ops {
    /// The matmul of the convolutions and of the GRU
    pub fn conv_gemm(&self) -> Gemm {
        if self.coop { Gemm::Coop } else if self.own_gemm { Gemm::Own } else { Gemm::Burn }
    }

    /// The matmul of the transformer's linear layers
    pub fn linear_gemm(&self) -> Gemm {
        if self.coop { Gemm::Coop } else if self.gemm_linear { Gemm::Own } else { Gemm::Burn }
    }
}

/// A convolution of this model: 1x1 ones and the plain path through burn, the others as im2col + matmul if asked; all
/// of them (but the flow head's last with `fused`) by coop.rs with `coop`
fn conv_fwd(conv: &Conv2d, x: Tensor<4>, ops: Ops) -> Tensor<4> {
    let [kh, kw] = conv.kernel_size;
    // Few output channels (the flow head's last, 256 -> 2): a direct kernel rather than columns 9 times the input for a
    // matmul of 2 rows
    if ops.fused && conv.weight.val().dims()[0] < 32 && kh * kw > 1 && conv.stride == [1, 1] && conv.dilation == [1, 1] && conv.groups == 1 {
        if let PaddingConfig2d::Explicit(t, l, b, r) = conv.padding {
            if let Some(bias) = &conv.bias {
                if t == b && l == r {
                    return kernels::conv_small(x, conv.weight.val(), bias.val(), t, l);
                }
            }
        }
    }
    if ops.coop && conv.stride[0] == conv.stride[1] && conv.dilation == [1, 1] && conv.groups == 1 {
        // The 1x1 ones are made without padding (Valid)
        let padding = match conv.padding {
            PaddingConfig2d::Explicit(t, l, b, r) if t == b && l == r => Some([t, l]),
            PaddingConfig2d::Valid => Some([0, 0]),
            _ => None,
        };
        if let Some(padding) = padding {
            return coop::conv2d(x, conv.weight.val(), conv.bias.as_ref().map(|b| b.val()), conv.stride[0], padding, coop::wgsl::COOP_DEFAULT);
        }
    }
    // 1x1 convolutions by the implicit GEMM kernel too with the own matmul: burn's convolution autotunes its matmul at the
    // first pass, which compiled and timed candidate kernels for minutes on an NVIDIA GPU through Vulkan (a Tesla T4: 190 of
    // the 211 s of a first start)
    if ops.own_gemm && kh == 1 && kw == 1 && conv.stride[0] == conv.stride[1] && conv.dilation == [1, 1] && conv.groups == 1 {
        let padding = match conv.padding {
            PaddingConfig2d::Explicit(t, l, b, r) if t == b && l == r => Some([t, l]),
            PaddingConfig2d::Valid => Some([0, 0]),
            _ => None,
        };
        if let Some(padding) = padding {
            return kernels::conv2d_implicit(x, conv.weight.val(), conv.bias.as_ref().map(|b| b.val()), conv.stride[0], padding);
        }
    }
    if !ops.conv_im2col || (kh == 1 && kw == 1) || conv.stride[0] != conv.stride[1] { return conv.forward(x); }
    match conv.padding {
        PaddingConfig2d::Explicit(t, l, b, r) if t == b && l == r => {
            if ops.implicit && ops.own_gemm && conv.weight.val().dims()[0] >= 32 {
                kernels::conv2d_implicit(x, conv.weight.val(), conv.bias.as_ref().map(|b| b.val()), conv.stride[0], [t, l])
            } else {
                kernels::conv2d_im2col(x, conv.weight.val(), conv.bias.as_ref().map(|b| b.val()), conv.stride[0], [t, l], IM2COL_MAX_BYTES, ops.own_gemm)
            }
        }
        _ => conv.forward(x),
    }
}

/// relu(conv_fwd(..)): with coop and `fused` the relu (and the bias) taken as coop.rs writes the convolution's output
/// (coop::conv2d_act), not as a pass of their own
fn conv_fwd_relu(conv: &Conv2d, x: Tensor<4>, ops: Ops) -> Tensor<4> {
    if ops.coop && ops.fused && conv.stride[0] == conv.stride[1] && conv.dilation == [1, 1] && conv.groups == 1 {
        let padding = match conv.padding {
            PaddingConfig2d::Explicit(t, l, b, r) if t == b && l == r => Some([t, l]),
            PaddingConfig2d::Valid => Some([0, 0]),
            _ => None,
        };
        if let Some(padding) = padding {
            return coop::conv2d_act(x, conv.weight.val(), conv.bias.as_ref().map(|b| b.val()), conv.stride[0], padding, true, coop::wgsl::COOP_DEFAULT);
        }
    }
    relu(conv_fwd(conv, x, ops))
}

/// The padding of a convolution that coop::conv2d_act_cat runs on a concatenation of two maps (coop and `fused`, a
/// gathered convolution of symmetric padding), or None
fn cat_padding(conv: &Conv2d, ops: Ops) -> Option<[usize; 2]> {
    if !(ops.coop && ops.fused && conv.stride[0] == conv.stride[1] && conv.dilation == [1, 1] && conv.groups == 1) {
        return None;
    }
    match conv.padding {
        PaddingConfig2d::Explicit(t, l, b, r) if t == b && l == r => Some([t, l]),
        PaddingConfig2d::Valid => Some([0, 0]),
        _ => None,
    }
}

fn conv(channels: [usize; 2], kernel: [usize; 2], stride: usize, bias: bool, device: &Device) -> Conv2d {
    let (ph, pw) = (kernel[0] / 2, kernel[1] / 2);
    Conv2dConfig::new(channels, kernel)
        .with_stride([stride, stride])
        .with_padding(PaddingConfig2d::Explicit(ph, pw, ph, pw))
        .with_bias(bias)
        .init(device)
}

fn instance_norm(channels: usize, device: &Device) -> InstanceNorm {
    // The defaults of nn.InstanceNorm2d: no affine parameters, eps 1e-5 (kernels::IN_EPS, which instance_norm_kernel
    // uses; burn adds it as an f32), statistics of each image
    InstanceNormConfig::new(channels).with_affine(false).with_epsilon(f64::from(kernels::IN_EPS)).init(device)
}

/// An InstanceNorm of the CNN, then relu with relu_y, then with res relu(res + y) (the add and relu that end a residual
/// block): with `fused` in one launch (kernels::instance_norm), else burn's InstanceNorm and the activations
fn norm_act(n: &InstanceNorm, x: Tensor<4>, relu_y: bool, res: Option<Tensor<4>>, fused: bool) -> Tensor<4> {
    if fused {
        return kernels::instance_norm(x, relu_y, res);
    }
    let y = n.forward(x);
    let y = if relu_y { relu(y) } else { y };
    match res {
        Some(r) => relu(r + y),
        None => y,
    }
}

// ---------------------------------------------------------------- CNN (unimatch backbone.py)

#[derive(Module, Debug)]
pub struct ResidualBlock {
    pub conv1: Conv2d,
    pub conv2: Conv2d,
    /// The 1x1 projection of the shortcut where the block changes the channels or the stride (`downsample.0` in unimatch)
    pub downsample: Option<Conv2d>,
    pub norm: InstanceNorm,
}

impl ResidualBlock {
    fn new(in_planes: usize, planes: usize, stride: usize, device: &Device) -> Self {
        let downsample = (stride != 1 || in_planes != planes).then(|| {
            Conv2dConfig::new([in_planes, planes], [1, 1]).with_stride([stride, stride]).with_bias(true).init(device)
        });
        Self {
            conv1: conv([in_planes, planes], [3, 3], stride, false, device),
            conv2: conv([planes, planes], [3, 3], 1, false, device),
            downsample,
            norm: instance_norm(planes, device),
        }
    }

    fn forward(&self, x: Tensor<4>, ops: Ops) -> Tensor<4> {
        let y = norm_act(&self.norm, conv_fwd(&self.conv1, x.clone(), ops), true, None, ops.fused);
        let x = match &self.downsample {
            Some(d) => norm_act(&self.norm, conv_fwd(d, x, ops), false, None, ops.fused),
            None => x,
        };
        // relu(x + relu(norm(conv2(y))))
        norm_act(&self.norm, conv_fwd(&self.conv2, y, ops), true, Some(x), ops.fused)
    }
}

/// One 3x3 convolution applied at stride 1 and stride 2 with the same weights (MultiScaleTridentConv, no bias)
#[derive(Module, Debug)]
pub struct TridentConv {
    pub weight: Param<Tensor<4>>,
}

#[derive(Module, Debug)]
pub struct CnnEncoder {
    pub conv1: Conv2d,
    pub norm1: InstanceNorm,
    pub layer1: Vec<ResidualBlock>,
    pub layer2: Vec<ResidualBlock>,
    pub layer3: Vec<ResidualBlock>,
    pub conv2: Conv2d,
    pub trident_conv: TridentConv,
}

impl CnnEncoder {
    fn new(device: &Device) -> Self {
        let layer = |a: usize, b: usize, stride: usize| vec![ResidualBlock::new(a, b, stride, device), ResidualBlock::new(b, b, 1, device)];
        let trident = Tensor::<4>::zeros([FEATURE_CHANNELS, FEATURE_CHANNELS, 3, 3], device);
        Self {
            conv1: conv([3, 64], [7, 7], 2, false, device),
            norm1: instance_norm(64, device),
            layer1: layer(64, 64, 1),
            layer2: layer(64, 96, 2),
            // Two output scales: the highest resolution stays at 1/4
            layer3: layer(96, 128, 1),
            conv2: Conv2dConfig::new([128, FEATURE_CHANNELS], [1, 1]).with_bias(true).init(device),
            trident_conv: TridentConv { weight: Param::from_tensor(trident) },
        }
    }

    /// Normalised images [N, 3, H, W] -> features at 1/4 and at 1/8, [N, 128, H/4, W/4] and [N, 128, H/8, W/8]
    pub fn forward(&self, x: Tensor<4>, ops: Ops) -> (Tensor<4>, Tensor<4>) {
        let mut x = norm_act(&self.norm1, conv_fwd(&self.conv1, x, ops), true, None, ops.fused);
        for block in self.layer1.iter().chain(&self.layer2).chain(&self.layer3) {
            x = block.forward(x, ops);
        }
        let x = conv_fwd(&self.conv2, x, ops);
        let w = self.trident_conv.weight.val();
        if ops.coop {
            let quarter = coop::conv2d(x.clone(), w.clone(), None, 1, [1, 1], coop::wgsl::COOP_DEFAULT);
            let eighth = coop::conv2d(x, w, None, 2, [1, 1], coop::wgsl::COOP_DEFAULT);
            return (quarter, eighth);
        }
        if ops.conv_im2col && ops.own_gemm && ops.implicit {
            let quarter = kernels::conv2d_implicit(x.clone(), w.clone(), None, 1, [1, 1]);
            let eighth = kernels::conv2d_implicit(x, w, None, 2, [1, 1]);
            return (quarter, eighth);
        }
        if ops.conv_im2col {
            let quarter = kernels::conv2d_im2col(x.clone(), w.clone(), None, 1, [1, 1], IM2COL_MAX_BYTES, ops.own_gemm);
            let eighth = kernels::conv2d_im2col(x, w, None, 2, [1, 1], IM2COL_MAX_BYTES, ops.own_gemm);
            return (quarter, eighth);
        }
        let quarter = conv2d(x.clone(), w.clone(), None, ConvOptions::new([1, 1], [1, 1], [1, 1], 1));
        let eighth = conv2d(x, w, None, ConvOptions::new([2, 2], [1, 1], [1, 1], 1));
        (quarter, eighth)
    }
}

// ---------------------------------------------------------------- Transformer (unimatch transformer.py, attention.py)

#[derive(Module, Debug)]
pub struct Mlp {
    /// Linear(256, 1024), `mlp.0` in unimatch
    pub fc1: Linear,
    /// Linear(1024, 128), `mlp.2`
    pub fc2: Linear,
}

#[derive(Module, Debug)]
pub struct TransformerLayer {
    pub q_proj: Linear,
    pub k_proj: Linear,
    pub v_proj: Linear,
    pub merge: Linear,
    pub norm1: LayerNorm,
    pub mlp: Option<Mlp>,
    pub norm2: Option<LayerNorm>,
}

impl TransformerLayer {
    fn new(with_ffn: bool, device: &Device) -> Self {
        let c = FEATURE_CHANNELS;
        let lin = |a: usize, b: usize| LinearConfig::new(a, b).with_bias(false).init(device);
        Self {
            q_proj: lin(c, c),
            k_proj: lin(c, c),
            v_proj: lin(c, c),
            merge: lin(c, c),
            norm1: LayerNormConfig::new(c).init(device),
            mlp: with_ffn.then(|| Mlp { fc1: lin(2 * c, 2 * c * 4), fc2: lin(2 * c * 4, c) }),
            norm2: with_ffn.then(|| LayerNormConfig::new(c).init(device)),
        }
    }

    /// source, target: [B, H*W, C]. `target_swapped`: the target is swap_halves(target) (only with coop and `fused`):
    /// k and v are projected from the target as given and the attention reads them from the other half of the batch
    /// (coop::window_attention_kv_swapped; the projections are per token, so they commute with the swap)
    #[allow(clippy::too_many_arguments)]
    fn forward(&self, source: Tensor<3>, target: Tensor<3>, target_swapped: bool, win: &Windows, with_shift: bool, gemm: Gemm, fused: bool) -> Tensor<3> {
        let q = lin(&self.q_proj, source.clone(), gemm);
        let k = lin(&self.k_proj, target.clone(), gemm);
        let v = lin(&self.v_proj, target, gemm);
        let message = if target_swapped {
            assert!(gemm == Gemm::Coop && fused, "a swapped target with coop and fused only");
            coop::window_attention_kv_swapped(q, k, v, win.h, win.w, win.splits, with_shift, coop::wgsl::COOP_DEFAULT)
        } else {
            split_window_attention(q, k, v, win, with_shift, fused, gemm == Gemm::Coop)
        };
        let message = lin(&self.merge, message, gemm);
        // source + the last norm's output, the add made by the norm with `fused`
        match (&self.mlp, &self.norm2) {
            (Some(mlp), Some(norm2)) => {
                let message = norm(&self.norm1, message, None, fused);
                // The hidden layer [B, L, 1024] of a bidirectional pass at 1/4 is 189 MB: a few images at a time, so that
                // no buffer is over IM2COL_MAX_BYTES (each token is on its own here)
                let [nb, l, _] = source.dims();
                let per_image = l * mlp.fc1.weight.val().dims()[1] * 4;
                let chunk = (IM2COL_MAX_BYTES / per_image).max(1);
                // With coop and `fused` fc1's matmul reads the source and the message where they are (lin_cat_in), not
                // their concatenation, and fc2's takes the GELU as it reads fc1's output (lin_gelu_in): neither is a pass
                // of its own
                let coop_fused = gemm == Gemm::Coop && fused;
                let ffn = |source: Tensor<3>, message: Tensor<3>| {
                    let hidden = if coop_fused {
                        lin_cat_in(&mlp.fc1, source.clone(), message)
                    } else {
                        lin(&mlp.fc1, Tensor::cat(vec![source.clone(), message], 2), gemm)
                    };
                    let out = if coop_fused { lin_gelu_in(&mlp.fc2, hidden) } else { lin(&mlp.fc2, gelu(hidden), gemm) };
                    norm(norm2, out, Some(source), fused)
                };
                if nb <= chunk {
                    ffn(source, message)
                } else {
                    Tensor::cat((0..nb).step_by(chunk).map(|s| {
                        let n = chunk.min(nb - s);
                        ffn(source.clone().narrow(0, s, n), message.clone().narrow(0, s, n))
                    }).collect(), 0)
                }
            }
            _ => norm(&self.norm1, message, Some(source), fused),
        }
    }
}

/// A LayerNorm of the transformer, plus a residual if given: kernels::layer_norm with `fused` (one launch; the model's
/// LayerNorms have the default epsilon, kernels::LN_EPS), burn's LayerNorm and an add otherwise (the reference)
fn norm(n: &LayerNorm, x: Tensor<3>, residual: Option<Tensor<3>>, fused: bool) -> Tensor<3> {
    if fused {
        let gamma = n.gamma.val();
        let beta = match &n.beta { Some(b) => b.val(), None => gamma.zeros_like() };
        return kernels::layer_norm(x, gamma, beta, residual);
    }
    let y = n.forward(x);
    match residual {
        Some(r) => r + y,
        None => y,
    }
}

/// a [B, M, K] @ b [B, K, N]: coop::gemm with Coop, kernels::gemm with Own (burn's matmul autotunes at the first pass:
/// minutes of compiling candidate kernels on an NVIDIA GPU through Vulkan), burn's matmul otherwise
fn bmm(a: Tensor<3>, b: Tensor<3>, gemm: Gemm) -> Tensor<3> {
    match gemm {
        Gemm::Coop => coop::gemm(a, b, coop::wgsl::COOP_DEFAULT),
        Gemm::Own => kernels::gemm(a, b, None),
        Gemm::Burn => a.matmul(b),
    }
}

/// A linear layer on [B, L, C]: burn's, or kernels::gemm or coop::gemm on the B * L rows at once (as burn folds them)
fn lin(l: &Linear, x: Tensor<3>, gemm: Gemm) -> Tensor<3> {
    if gemm == Gemm::Burn {
        return l.forward(x);
    }
    let [b, n, k] = x.dims();
    let w = l.weight.val(); // [d_input, d_output]
    let d_out = w.dims()[1];
    let (x, w) = (x.reshape([1, b * n, k]), w.unsqueeze_dim::<3>(0));
    let out = if gemm == Gemm::Coop { coop::gemm(x, w, coop::wgsl::COOP_DEFAULT) } else { kernels::gemm(x, w, None) };
    let out = out.reshape([b, n, d_out]);
    match &l.bias {
        Some(bias) => out + bias.val().reshape([1, 1, d_out]),
        None => out,
    }
}

/// lin of cat([x1, x2], 2) with coop, without the concatenation (coop::gemm_cat_a): x1 [B, L, C1], x2 [B, L, C2]
fn lin_cat_in(l: &Linear, x1: Tensor<3>, x2: Tensor<3>) -> Tensor<3> {
    let [b, n, k1] = x1.dims();
    let k2 = x2.dims()[2];
    let w = l.weight.val(); // [d_input, d_output]
    let d_out = w.dims()[1];
    let out = coop::gemm_cat_a(x1.reshape([1, b * n, k1]), x2.reshape([1, b * n, k2]), w.unsqueeze_dim::<3>(0), coop::wgsl::COOP_DEFAULT).reshape([b, n, d_out]);
    match &l.bias {
        Some(bias) => out + bias.val().reshape([1, 1, d_out]),
        None => out,
    }
}

/// lin(l, gelu(x), Coop) with the GELU taken as coop.rs's matmul reads x (coop::gemm_gelu_a)
fn lin_gelu_in(l: &Linear, x: Tensor<3>) -> Tensor<3> {
    let [b, n, k] = x.dims();
    let w = l.weight.val(); // [d_input, d_output]
    let d_out = w.dims()[1];
    let out = coop::gemm_gelu_a(x.reshape([1, b * n, k]), w.unsqueeze_dim::<3>(0), coop::wgsl::COOP_DEFAULT).reshape([b, n, d_out]);
    match &l.bias {
        Some(bias) => out + bias.val().reshape([1, 1, d_out]),
        None => out,
    }
}

#[derive(Module, Debug)]
pub struct TransformerBlock {
    pub self_attn: TransformerLayer,
    pub cross_attn_ffn: TransformerLayer,
}

#[derive(Module, Debug)]
pub struct FeatureTransformer {
    pub layers: Vec<TransformerBlock>,
}

impl FeatureTransformer {
    fn new(device: &Device) -> Self {
        Self { layers: (0..6).map(|_| TransformerBlock { self_attn: TransformerLayer::new(false, device), cross_attn_ffn: TransformerLayer::new(true, device) }).collect() }
    }

    /// feature0, feature1: [B, C, H, W] (position added) -> the same, after self- and cross-attention
    pub fn forward(&self, feature0: Tensor<4>, feature1: Tensor<4>, win: &Windows, gemm: Gemm, fused: bool) -> (Tensor<4>, Tensor<4>) {
        let [b, c, h, w] = feature0.dims();
        let tokens = |f: Tensor<4>| f.reshape([b, c, h * w]).swap_dims(1, 2);
        let (f0, f1) = (tokens(feature0), tokens(feature1));
        // concat1 of unimatch, the halves of concat0 swapped, is made where it is read
        let mut concat0 = Tensor::cat(vec![f0, f1], 0);
        for (i, layer) in self.layers.iter().enumerate() {
            let with_shift = i % 2 == 1;
            // With coop and `fused` the cross-attention reads the keys and values of the other image where they are
            let swapped_in_kernel = gemm == Gemm::Coop && fused;
            let concat1 = if swapped_in_kernel { concat0.clone() } else { swap_halves(concat0.clone()) };
            let s = layer.self_attn.forward(concat0.clone(), concat0, false, win, with_shift, gemm, fused);
            concat0 = layer.cross_attn_ffn.forward(s, concat1, swapped_in_kernel, win, with_shift, gemm, fused);
        }
        let back = |t: Tensor<3>| t.swap_dims(1, 2).reshape([b, c, h, w]);
        (back(concat0.clone().narrow(0, 0, b)), back(concat0.narrow(0, b, b)))
    }
}

/// [2B, ...] -> the second half first
pub fn swap_halves<const D: usize>(t: Tensor<D>) -> Tensor<D> {
    let b = t.dims()[0] / 2;
    Tensor::cat(vec![t.clone().narrow(0, b, b), t.narrow(0, 0, b)], 0)
}

/// What split-window attention at one feature size needs that depends only on the size: the window layout, the sine
/// position embedding of a window tiled over the map, and the attention mask of the shifted windows
pub struct Windows {
    pub h: usize,
    pub w: usize,
    pub splits: usize,
    /// [1, C, H, W]
    pub position: Tensor<4>,
    /// [K*K, L, L], 0 or -100, L = (H/K)*(W/K)
    pub shift_mask: Tensor<3>,
}

impl Windows {
    pub fn new(h: usize, w: usize, splits: usize, device: &Device) -> Self {
        assert!(h % splits == 0 && w % splits == 0, "{h}x{w} does not split into {splits}x{splits} windows");
        let (wh, ww) = (h / splits, w / splits);
        // PositionEmbeddingSine(num_pos_feats = 64, temperature 10000, normalize, scale 2 pi) of one window, in float32 as
        // unimatch computes it; every window gets the same (feature_add_position adds it per split)
        let half = FEATURE_CHANNELS / 2;
        let scale = 2.0f32 * std::f32::consts::PI;
        let eps = 1e-6f32;
        let dim_t: Vec<f32> = (0..half).map(|i| 10000f32.powf((2 * (i / 2)) as f32 / half as f32)).collect();
        let mut pos = vec![0f32; FEATURE_CHANNELS * h * w];
        for y in 0..h {
            for x in 0..w {
                let ye = ((y % wh) + 1) as f32 / (wh as f32 + eps) * scale;
                let xe = ((x % ww) + 1) as f32 / (ww as f32 + eps) * scale;
                for i in 0..half {
                    let (py, px) = (ye / dim_t[i], xe / dim_t[i]);
                    // Channels: pos_y (sin of the even, cos of the odd, interleaved), then pos_x
                    let (vy, vx) = if i % 2 == 0 { (py.sin(), px.sin()) } else { (py.cos(), px.cos()) };
                    pos[i * h * w + y * w + x] = vy;
                    pos[(half + i) * h * w + y * w + x] = vx;
                }
            }
        }
        let position = Tensor::<4>::from_data(TensorData::new(pos, [1, FEATURE_CHANNELS, h, w]), device);

        // generate_shift_window_attn_mask: regions of the map (3 x 3, split at the shifted window borders), windows of the
        // map, -100 between two positions of a window that are from different regions
        let (sh, sw) = (wh / 2, ww / 2);
        let region = |v: usize, n: usize, win: usize, shift: usize| if v < n - win { 0 } else if v < n - shift { 1 } else { 2 };
        let l = wh * ww;
        let mut mask = vec![0f32; splits * splits * l * l];
        for wy in 0..splits {
            for wx in 0..splits {
                let labels: Vec<usize> = (0..l).map(|p| {
                    let (y, x) = (wy * wh + p / ww, wx * ww + p % ww);
                    region(y, h, wh, sh) * 3 + region(x, w, ww, sw)
                }).collect();
                let base = (wy * splits + wx) * l * l;
                for i in 0..l {
                    for j in 0..l {
                        if labels[i] != labels[j] { mask[base + i * l + j] = -100.0; }
                    }
                }
            }
        }
        let shift_mask = Tensor::<3>::from_data(TensorData::new(mask, [splits * splits, l, l]), device);
        Self { h, w, splits, position, shift_mask }
    }
}

/// [B, H, W, C] -> [B*K*K, (H/K)*(W/K), C], windows in row-major order (unimatch split_feature, channel last)
fn to_windows(x: Tensor<4>, k: usize) -> Tensor<3> {
    let [b, h, w, c] = x.dims();
    x.reshape([b * k, h / k, k, w / k * c])
        .swap_dims(1, 2) // [b*k, k, h/k, (w/k)*c]: window row, window column, ...
        .reshape([b * k * k, (h / k) * (w / k), c])
}

/// The inverse of `to_windows`
fn from_windows(x: Tensor<3>, b: usize, h: usize, w: usize, k: usize) -> Tensor<4> {
    let c = x.dims()[2];
    x.reshape([b * k, k, h / k, w / k * c]).swap_dims(1, 2).reshape([b, h, w, c])
}

/// unimatch single_head_split_window_attention: q, k, v [B, H*W, C] -> [B, H*W, C]; with `fused`, kernels::window_attention
pub fn split_window_attention(q: Tensor<3>, k: Tensor<3>, v: Tensor<3>, win: &Windows, with_shift: bool, fused: bool, coop: bool) -> Tensor<3> {
    // fused with coop: the same three launches, the two matmuls on cooperative matrices (coop.rs)
    if fused && coop {
        return coop::window_attention(q, k, v, win.h, win.w, win.splits, with_shift, coop::wgsl::COOP_DEFAULT);
    }
    if fused {
        return kernels::window_attention(q, k, v, win.h, win.w, win.splits, with_shift);
    }
    let [b, _, c] = q.dims();
    let (h, w, splits) = (win.h, win.w, win.splits);
    let (sh, sw) = (h / splits / 2, w / splits / 2);
    let grid = |t: Tensor<3>| {
        let t = t.reshape([b, h, w, c]);
        let t = if with_shift { t.roll_dim(-(sh as i64), 1).roll_dim(-(sw as i64), 2) } else { t };
        to_windows(t, splits)
    };
    let (q, k, v) = (grid(q), grid(k), grid(v));
    let mut scores = q.matmul(k.swap_dims(1, 2)).div_scalar((c as f32).sqrt());
    if with_shift {
        let n = splits * splits;
        let l = scores.dims()[1];
        scores = (scores.reshape([b, n, l, l]) + win.shift_mask.clone().unsqueeze_dim::<4>(0)).reshape([b * n, l, l]);
    }
    let out = softmax(scores, 2).matmul(v);
    let out = from_windows(out, b, h, w, splits);
    let out = if with_shift { out.roll_dim(sh as i64, 1).roll_dim(sw as i64, 2) } else { out };
    out.reshape([b, h * w, c])
}

// ---------------------------------------------------------------- Matching and propagation (matching.py, attention.py)

/// [1, 2, H, W]: x then y of each position
fn coords(h: usize, w: usize, device: &Device) -> Tensor<4> {
    let mut v = vec![0f32; 2 * h * w];
    for y in 0..h {
        for x in 0..w {
            v[y * w + x] = x as f32;
            v[h * w + y * w + x] = y as f32;
        }
    }
    Tensor::from_data(TensorData::new(v, [1, 2, h, w]), device)
}

/// The window offsets of radius r in the order of unimatch (generate_window_grid, reshaped): k = (dy + r) * (2r + 1) + (dx + r)
fn offsets(r: usize) -> Vec<(i32, i32)> {
    let n = 2 * r as i32 + 1;
    (0..n * n).map(|k| (k % n - r as i32, k / n - r as i32)).collect()
}

/// global_correlation_softmax: [B, C, H, W] x 2 -> flow [B, 2, H, W]; bidirectional: [2B, 2, H, W], the flow 1 -> 0 from
/// the transposed correlation. With `fused` the softmax and its matmul with the grid in one launch (kernels::softmax_flow)
pub fn global_correlation_softmax(f0: Tensor<4>, f1: Tensor<4>, bidirectional: bool, gemm: Gemm, fused: bool) -> Tensor<4> {
    let [b, c, h, w] = f0.dims();
    let device = f0.device();
    let f0 = f0.reshape([b, c, h * w]).swap_dims(1, 2); // [B, HW, C]
    let f1 = f1.reshape([b, c, h * w]); // [B, C, HW]
    let corr = bmm(f0, f1, gemm).div_scalar((c as f32).sqrt());
    let (corr, b) = if bidirectional { (Tensor::cat(vec![corr.clone(), corr.swap_dims(1, 2)], 0), 2 * b) } else { (corr, b) };
    let grid = coords(h, w, &device); // [1, 2, H, W]
    let grid_t = grid.clone().reshape([1, 2, h * w]).swap_dims(1, 2); // [1, HW, 2]
    let correspondence = if fused { kernels::softmax_flow(corr, grid_t) } else { softmax(corr, 2).matmul(grid_t.repeat_dim(0, b)) }; // [B, HW, 2]
    let correspondence = correspondence.swap_dims(1, 2).reshape([b, 2, h, w]);
    correspondence - grid
}

/// The (2r+1)^2 copies of x moved by the window offsets (the value at p + offset), zero outside: one [B, C, H, W] each
pub fn shifted(x: Tensor<4>, r: usize) -> Vec<Tensor<4>> {
    let [_, _, h, w] = x.dims();
    let padded = x.pad([(r, r), (r, r)], PadMode::Constant(0.0));
    offsets(r).into_iter().map(|(dx, dy)| {
        let (y0, x0) = ((r as i32 + dy) as usize, (r as i32 + dx) as usize);
        padded.clone().narrow(2, y0, h).narrow(3, x0, w)
    }).collect()
}

/// local_correlation_softmax (integer offsets, radius r): flow [B, 2, H, W]. Out-of-image offsets get -1e9 before the
/// softmax, as in unimatch
pub fn local_correlation_softmax(f0: Tensor<4>, f1: Tensor<4>, r: usize) -> Tensor<4> {
    let [_, c, h, w] = f0.dims();
    let device = f0.device();
    let scale = 1.0 / (c as f32).sqrt();
    let corr: Vec<Tensor<4>> = shifted(f1, r).into_iter().map(|s| (f0.clone() * s).sum_dim(1).mul_scalar(scale)).collect();
    let corr = Tensor::cat(corr, 1); // [B, (2r+1)^2, H, W]
    let n = (2 * r + 1) * (2 * r + 1);
    let mut invalid = vec![0f32; n * h * w];
    let mut sx_all = vec![0f32; n * h * w];
    let mut sy_all = vec![0f32; n * h * w];
    for (k, (dx, dy)) in offsets(r).into_iter().enumerate() {
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (x as i32 + dx, y as i32 + dy);
                let i = k * h * w + y * w + x;
                if sx < 0 || sy < 0 || sx >= w as i32 || sy >= h as i32 { invalid[i] = -1e9; }
                sx_all[i] = sx as f32;
                sy_all[i] = sy as f32;
            }
        }
    }
    let t = |v: Vec<f32>| Tensor::<4>::from_data(TensorData::new(v, [1, n, h, w]), &device);
    let prob = softmax(corr + t(invalid), 1);
    // The sample coordinates weighted by the probabilities, then the position subtracted (as unimatch: not the offsets)
    let cx = (prob.clone() * t(sx_all)).sum_dim(1);
    let cy = (prob * t(sy_all)).sum_dim(1);
    Tensor::cat(vec![cx, cy], 1) - coords(h, w, &device)
}

/// Bilinear samples of x at pixel positions [B, 2, H, W] (x then y), zeros outside, as F.grid_sample with align_corners
/// of the coordinates normalised the way unimatch does: (p - c) / c, c = (size - 1) / 2
fn sample(x: Tensor<4>, positions: Tensor<4>) -> Tensor<4> {
    let [_, _, h, w] = x.dims();
    let [b, _, ph, pw] = positions.dims();
    let cx = ((w - 1) as f32) / 2.0;
    let cy = ((h - 1) as f32) / 2.0;
    // [B, H, W, 2] laid out as such: the grid_sample kernel of burn-cubecl 0.22 reads y right after x (offset + 1), it
    // does not follow the strides of a permuted view
    let nx = positions.clone().narrow(1, 0, 1).sub_scalar(cx).div_scalar(cx).reshape([b, ph, pw, 1]);
    let ny = positions.narrow(1, 1, 1).sub_scalar(cy).div_scalar(cy).reshape([b, ph, pw, 1]);
    let grid = Tensor::cat(vec![nx, ny], 3);
    x.grid_sample_2d(grid, GridSampleOptions::new(InterpolateMode::Bilinear).with_padding_mode(GridSamplePaddingMode::Zeros).with_align_corners(true))
}

/// flow_warp: feature [B, C, H, W] sampled at each position + flow. bilinear_sample normalises with 2 x / (w - 1) - 1,
/// the same as `sample`
pub fn flow_warp(feature: Tensor<4>, flow: Tensor<4>) -> Tensor<4> {
    let [_, _, h, w] = feature.dims();
    let positions = coords(h, w, &feature.device()) + flow;
    sample(feature, positions)
}

/// local_correlation_with_flow, radius r: [B, (2r+1)^2, H, W]
pub fn local_correlation_with_flow(f0: Tensor<4>, f1: Tensor<4>, flow: Tensor<4>, r: usize) -> Tensor<4> {
    let [_, c, h, w] = f0.dims();
    let device = f0.device();
    let scale = 1.0 / (c as f32).sqrt();
    let base = coords(h, w, &device) + flow;
    let corr: Vec<Tensor<4>> = offsets(r).into_iter().map(|(dx, dy)| {
        let offset = Tensor::<4>::from_data(TensorData::new(vec![dx as f32, dy as f32], [1, 2, 1, 1]), &device);
        let s = sample(f1.clone(), base.clone() + offset);
        (f0.clone() * s).sum_dim(1).mul_scalar(scale)
    }).collect();
    Tensor::cat(corr, 1)
}

#[derive(Module, Debug)]
pub struct SelfAttnPropagation {
    pub q_proj: Linear,
    pub k_proj: Linear,
}

impl SelfAttnPropagation {
    fn new(device: &Device) -> Self {
        let c = FEATURE_CHANNELS;
        Self { q_proj: LinearConfig::new(c, c).init(device), k_proj: LinearConfig::new(c, c).init(device) }
    }

    /// Global: feature0 [B, C, H, W], flow [B, 2, H, W] -> flow. The key is k_proj of the projected query, as unimatch
    /// computes it (its own note: an accident of the training code, kept). `gemm`: the projections and the scores by
    /// coop.rs with Coop, burn's otherwise; the softmax and its matmul with the flow (2 columns) in one launch with `fused`
    /// (kernels::softmax_flow), burn's otherwise
    fn global(&self, feature0: Tensor<4>, flow: Tensor<4>, gemm: Gemm, fused: bool) -> Tensor<4> {
        let [b, c, h, w] = feature0.dims();
        let g = gemm;
        let q = lin(&self.q_proj, feature0.reshape([b, c, h * w]).swap_dims(1, 2), g);
        let k = lin(&self.k_proj, q.clone(), g);
        let v = flow.reshape([b, 2, h * w]).swap_dims(1, 2);
        let scores = bmm(q, k.swap_dims(1, 2), g).div_scalar((c as f32).sqrt());
        let out = if fused { kernels::softmax_flow(scores, v) } else { softmax(scores, 2).matmul(v) };
        out.swap_dims(1, 2).reshape([b, 2, h, w])
    }

    /// Local window of radius r. The window comes from F.unfold with zero padding: a neighbour outside the map has a key of
    /// zeros (score 0, still in the softmax) and a flow of zeros
    fn local(&self, feature0: Tensor<4>, flow: Tensor<4>, r: usize, with_kernels: bool, gemm: Gemm) -> Tensor<4> {
        let [b, c, h, w] = feature0.dims();
        let g = gemm;
        let tokens = feature0.reshape([b, c, h * w]).swap_dims(1, 2);
        let as_map = |t: Tensor<3>| t.swap_dims(1, 2).reshape([b, c, h, w]);
        let q = as_map(lin(&self.q_proj, tokens.clone(), g));
        let k = as_map(lin(&self.k_proj, tokens, g));
        if with_kernels {
            assert_eq!(r, kernels::PROP_RADIUS);
            return kernels::local_propagation(q, k, flow);
        }
        local_propagation(q, k, flow, r)
    }
}

/// The plain form of the local window propagation, q and k projected: [B, C, H, W]
pub fn local_propagation(q: Tensor<4>, k: Tensor<4>, flow: Tensor<4>, r: usize) -> Tensor<4> {
    {
        let c = q.dims()[1];
        let scale = 1.0 / (c as f32).sqrt();
        let scores: Vec<Tensor<4>> = shifted(k, r).into_iter().map(|s| (q.clone() * s).sum_dim(1).mul_scalar(scale)).collect();
        let prob = softmax(Tensor::cat(scores, 1), 1); // [B, (2r+1)^2, H, W]
        let mut out: Option<Tensor<4>> = None;
        for (i, f) in shifted(flow, r).into_iter().enumerate() {
            let term = prob.clone().narrow(1, i, 1) * f;
            out = Some(match out { Some(o) => o + term, None => term });
        }
        out.expect("a window of at least one position")
    }
}

// ---------------------------------------------------------------- Refinement (reg_refine.py)

#[derive(Module, Debug)]
pub struct BasicMotionEncoder {
    pub convc1: Conv2d,
    pub convc2: Conv2d,
    pub convf1: Conv2d,
    pub convf2: Conv2d,
    pub conv: Conv2d,
}

#[derive(Module, Debug)]
pub struct SepConvGru {
    pub convz1: Conv2d,
    pub convr1: Conv2d,
    pub convq1: Conv2d,
    pub convz2: Conv2d,
    pub convr2: Conv2d,
    pub convq2: Conv2d,
}

#[derive(Module, Debug)]
pub struct FlowHead {
    pub conv1: Conv2d,
    pub conv2: Conv2d,
}

#[derive(Module, Debug)]
pub struct MaskHead {
    /// `mask.0` in unimatch
    pub conv1: Conv2d,
    /// `mask.2`
    pub conv2: Conv2d,
}

#[derive(Module, Debug)]
pub struct BasicUpdateBlock {
    pub encoder: BasicMotionEncoder,
    pub gru: SepConvGru,
    pub flow_head: FlowHead,
    pub mask: MaskHead,
}

impl BasicUpdateBlock {
    fn new(device: &Device) -> Self {
        let corr = (2 * LOCAL_RADIUS + 1) * (2 * LOCAL_RADIUS + 1);
        let gru_in = 128 + 256;
        Self {
            encoder: BasicMotionEncoder {
                convc1: conv([corr, 256], [1, 1], 1, true, device),
                convc2: conv([256, 192], [3, 3], 1, true, device),
                convf1: conv([2, 128], [7, 7], 1, true, device),
                convf2: conv([128, 64], [3, 3], 1, true, device),
                conv: conv([64 + 192, 128 - 2], [3, 3], 1, true, device),
            },
            gru: SepConvGru {
                convz1: conv([gru_in, 128], [1, 5], 1, true, device),
                convr1: conv([gru_in, 128], [1, 5], 1, true, device),
                convq1: conv([gru_in, 128], [1, 5], 1, true, device),
                convz2: conv([gru_in, 128], [5, 1], 1, true, device),
                convr2: conv([gru_in, 128], [5, 1], 1, true, device),
                convq2: conv([gru_in, 128], [5, 1], 1, true, device),
            },
            flow_head: FlowHead { conv1: conv([128, 256], [3, 3], 1, true, device), conv2: conv([256, 2], [3, 3], 1, true, device) },
            mask: MaskHead { conv1: conv([128, 256], [3, 3], 1, true, device), conv2: conv([256, UPSAMPLE * UPSAMPLE * 9], [1, 1], 1, true, device) },
        }
    }

    /// The motion features, cat([out, flow], 1) of unimatch, as its two parts (out, flow): with coop and `fused` the
    /// convolutions that read a concatenation gather its channels from the parts (coop::conv2d_act_cat), so it is not
    /// made
    fn motion(&self, flow: Tensor<4>, corr: Tensor<4>, ops: Ops) -> (Tensor<4>, Tensor<4>) {
        let e = &self.encoder;
        let cor = conv_fwd_relu(&e.convc2, conv_fwd_relu(&e.convc1, corr, ops), ops);
        let flo = conv_fwd_relu(&e.convf2, conv_fwd_relu(&e.convf1, flow.clone(), ops), ops);
        let out = match cat_padding(&e.conv, ops) {
            Some(padding) => coop::conv2d_act_cat(cor, flo, e.conv.weight.val(), e.conv.bias.as_ref().map(|b| b.val()), e.conv.stride[0], padding, true, coop::wgsl::COOP_DEFAULT),
            None => conv_fwd_relu(&e.conv, Tensor::cat(vec![cor, flo], 1), ops),
        };
        (out, flow)
    }

    fn gru(&self, h: Tensor<4>, x: Tensor<4>, ops: Ops) -> Tensor<4> {
        let g = &self.gru;
        let half = |h: Tensor<4>, cz: &Conv2d, cr: &Conv2d, cq: &Conv2d| {
            let hx = Tensor::cat(vec![h.clone(), x.clone()], 1);
            let z = sigmoid(conv_fwd(cz, hx.clone(), ops));
            let r = sigmoid(conv_fwd(cr, hx, ops));
            let q = tanh(conv_fwd(cq, Tensor::cat(vec![r * h.clone(), x.clone()], 1), ops));
            // (1 - z) * h + z * q, in the same order of operations as unimatch
            (z.clone().neg().add_scalar(1.0)) * h + z * q
        };
        let h = half(h, &g.convz1, &g.convr1, &g.convq1);
        half(h, &g.convz2, &g.convr2, &g.convq2)
    }

    /// One refinement step: the new hidden state and the flow update
    fn forward(&self, net: Tensor<4>, inp: Tensor<4>, corr: Tensor<4>, flow: Tensor<4>, ops: Ops, factored: Option<(&GruFactored, &GruConstants)>) -> (Tensor<4>, Tensor<4>) {
        let (out, flow) = self.motion(flow, corr, ops);
        let net = match factored {
            Some((g, c)) if ops.coop && ops.fused => g.step(c, net, out, Some(flow), ops.conv_gemm(), ops.implicit, ops.fused),
            Some((g, c)) => g.step(c, net, Tensor::cat(vec![out, flow], 1), None, ops.conv_gemm(), ops.implicit, ops.fused),
            None => self.gru(net, Tensor::cat(vec![inp, out, flow], 1), ops),
        };
        let delta = conv_fwd(&self.flow_head.conv2, conv_fwd_relu(&self.flow_head.conv1, net.clone(), ops), ops);
        (net, delta)
    }

    fn up_mask(&self, net: Tensor<4>, ops: Ops) -> Tensor<4> {
        conv_fwd(&self.mask.conv2, conv_fwd_relu(&self.mask.conv1, net, ops), ops)
    }
}

/// The convolutions of the GRU split by input channels, for `BasicUpdateBlock::forward_factored`. A GRU convolution reads
/// [h, inp, motion] (128 channels each; q reads [r * h, inp, motion]): a convolution is linear in its input, so it is the
/// sum of the convolutions of the three parts. Matrices [Cout, 128 * KH * KW] in im2col order, several outputs stacked
/// where they read the same columns.
pub struct GruFactored {
    zr1_h: Tensor<3>, // [1, 256, 640]: z1 and r1 from h (1x5)
    zr1_i: Tensor<3>, // z1 and r1 from inp
    zr1_b: Tensor<3>, // [1, 256, 1]
    q1_h: Tensor<3>,  // [1, 128, 640]
    q1_i: Tensor<3>,
    q1_b: Tensor<3>,
    zrq1_m: Tensor<3>, // [1, 384, 640]: z1, r1 and q1 from motion
    zr2_h: Tensor<3>,  // [1, 256, 640]: z2 and r2 from h (5x1)
    q2_h: Tensor<3>,
    zrq2_i: Tensor<3>, // [1, 384, 640]
    zrq2_b: Tensor<3>,
    zrq2_m: Tensor<3>,
}

/// The weight of a GRU convolution [128, 384, KH, KW] as [1, 128, 128 * KH * KW] for one of its three inputs (0 h, 1 inp,
/// 2 motion), and its bias as [1, 128, 1]
fn gru_part(conv: &Conv2d, part: usize) -> Tensor<3> {
    let [cout, cin, kh, kw] = conv.weight.val().dims();
    let k = kh * kw;
    conv.weight.val().reshape([1, cout, cin * k]).narrow(2, part * 128 * k, 128 * k)
}
fn gru_bias(conv: &Conv2d) -> Tensor<3> {
    let b = conv.bias.as_ref().expect("GRU convolutions have a bias").val();
    let n = b.dims()[0];
    b.reshape([1, n, 1])
}

impl BasicUpdateBlock {
    pub fn factor_gru(&self) -> GruFactored {
        let g = &self.gru;
        let cat = |v: Vec<Tensor<3>>| Tensor::cat(v, 1);
        GruFactored {
            zr1_h: cat(vec![gru_part(&g.convz1, 0), gru_part(&g.convr1, 0)]),
            zr1_i: cat(vec![gru_part(&g.convz1, 1), gru_part(&g.convr1, 1)]),
            zr1_b: cat(vec![gru_bias(&g.convz1), gru_bias(&g.convr1)]),
            q1_h: gru_part(&g.convq1, 0),
            q1_i: gru_part(&g.convq1, 1),
            q1_b: gru_bias(&g.convq1),
            zrq1_m: cat(vec![gru_part(&g.convz1, 2), gru_part(&g.convr1, 2), gru_part(&g.convq1, 2)]),
            zr2_h: cat(vec![gru_part(&g.convz2, 0), gru_part(&g.convr2, 0)]),
            q2_h: gru_part(&g.convq2, 0),
            zrq2_i: cat(vec![gru_part(&g.convz2, 1), gru_part(&g.convr2, 1), gru_part(&g.convq2, 1)]),
            zrq2_b: cat(vec![gru_bias(&g.convz2), gru_bias(&g.convr2), gru_bias(&g.convq2)]),
            zrq2_m: cat(vec![gru_part(&g.convz2, 2), gru_part(&g.convr2, 2), gru_part(&g.convq2, 2)]),
        }
    }
}

/// w [1, Co, K] @ cols [N, K, L], by burn, kernels::gemm or coop::gemm
fn mm(w: Tensor<3>, cols: Tensor<3>, gemm: Gemm) -> Tensor<3> {
    match gemm {
        Gemm::Burn => w.matmul(cols),
        Gemm::Own => kernels::gemm(w, cols, None),
        Gemm::Coop => coop::gemm(w, cols, coop::wgsl::COOP_DEFAULT),
    }
}

/// Columns of x [N, C, H, W] for a 1x5 (horizontal) or 5x1 (vertical) convolution with "same" padding: [N, C*5, H*W]
fn gru_cols(x: Tensor<4>, horizontal: bool) -> Tensor<3> {
    let (kh, kw, ph, pw) = if horizontal { (1, 5, 0, 2) } else { (5, 1, 2, 0) };
    kernels::im2col(x, kh, kw, 1, ph, pw)
}

/// Whether the GRU's convolutions gather their columns in the matmul (mmc) rather than read them from im2col
fn gathers(gemm: Gemm, implicit: bool) -> bool {
    gemm == Gemm::Coop || (implicit && gemm == Gemm::Own)
}

/// w [1, Co, C*5] @ the 1x5 (horizontal) or 5x1 columns of x: the columns gathered in the matmul by coop::conv_gemm,
/// or by kernels::conv_gemm with `implicit` and kernels::gemm
fn mmc(w: Tensor<3>, x: Tensor<4>, horizontal: bool, gemm: Gemm, implicit: bool) -> Tensor<3> {
    let (kh, kw, ph, pw) = if horizontal { (1, 5, 0, 2) } else { (5, 1, 2, 0) };
    if gemm == Gemm::Coop {
        return coop::conv_gemm(w, x, kh, kw, 1, ph, pw, coop::wgsl::COOP_DEFAULT);
    }
    if gathers(gemm, implicit) {
        return kernels::conv_gemm(w, x, kh, kw, 1, ph, pw);
    }
    mm(w, gru_cols(x, horizontal), gemm)
}

/// mmc of cat([x, tail], 1); with coop the columns gathered from both (coop::conv_gemm_cat), not concatenated
fn mmc_tail(w: Tensor<3>, x: Tensor<4>, tail: Option<Tensor<4>>, horizontal: bool, gemm: Gemm, implicit: bool) -> Tensor<3> {
    match tail {
        Some(t) if gemm == Gemm::Coop => {
            let (kh, kw, ph, pw) = if horizontal { (1, 5, 0, 2) } else { (5, 1, 2, 0) };
            coop::conv_gemm_cat(w, x, t, kh, kw, 1, ph, pw, coop::wgsl::COOP_DEFAULT)
        }
        Some(t) => mmc(w, Tensor::cat(vec![x, t], 1), horizontal, gemm, implicit),
        None => mmc(w, x, horizontal, gemm, implicit),
    }
}

/// What the GRU of every refinement step shares: h0 and inp are the same in all six (unimatch computes them again each
/// time), so their parts of the convolutions are computed once. [N, C, H*W]
pub struct GruConstants {
    zr1: Tensor<3>,  // z1, r1 from h0 and inp, with the biases
    q1: Tensor<3>,   // q1 from inp, with its bias
    zrq2: Tensor<3>, // z2, r2, q2 from inp, with the biases
}

impl GruFactored {
    pub fn constants(&self, h0: Tensor<4>, inp: Tensor<4>, gemm: Gemm, implicit: bool) -> GruConstants {
        if gathers(gemm, implicit) {
            return GruConstants {
                zr1: mmc(self.zr1_h.clone(), h0, true, gemm, implicit) + mmc(self.zr1_i.clone(), inp.clone(), true, gemm, implicit) + self.zr1_b.clone(),
                q1: mmc(self.q1_i.clone(), inp.clone(), true, gemm, implicit) + self.q1_b.clone(),
                zrq2: mmc(self.zrq2_i.clone(), inp, false, gemm, implicit) + self.zrq2_b.clone(),
            };
        }
        let ci1 = gru_cols(inp.clone(), true);
        let ci2 = gru_cols(inp, false);
        let ch1 = gru_cols(h0, true);
        GruConstants {
            zr1: mm(self.zr1_h.clone(), ch1, gemm) + mm(self.zr1_i.clone(), ci1.clone(), gemm) + self.zr1_b.clone(),
            q1: mm(self.q1_i.clone(), ci1, gemm) + self.q1_b.clone(),
            zrq2: mm(self.zrq2_i.clone(), ci2, gemm) + self.zrq2_b.clone(),
        }
    }

    /// SepConvGRU(h0, [inp, motion]), the same as `BasicUpdateBlock::gru` up to the order of the sums; with `tail` the
    /// motion is cat([motion, tail], 1), its convolutions gathering from both with coop (coop::conv_gemm_cat)
    #[allow(clippy::too_many_arguments)]
    pub fn step(&self, c: &GruConstants, h0: Tensor<4>, motion: Tensor<4>, tail: Option<Tensor<4>>, gemm: Gemm, implicit: bool, fused: bool) -> Tensor<4> {
        let (motion, tail) = match tail {
            Some(t) if gemm != Gemm::Coop => (Tensor::cat(vec![motion, t], 1), None),
            tail => (motion, tail),
        };
        let mmc_motion = |w: Tensor<3>, horizontal: bool| mmc_tail(w, motion.clone(), tail.clone(), horizontal, gemm, implicit);
        let [n, ch, hh, ww] = h0.dims();
        let flat = |t: Tensor<4>| t.reshape([n, ch, hh * ww]);
        let map = |t: Tensor<3>| t.reshape([n, ch, hh, ww]);
        if fused {
            // The same sums, the gates by kernels::gru_gate: r * h, then the blend with tanh(q), for each half
            use kernels::QFrom;
            let h0f = flat(h0);
            let m1 = mmc_motion(self.zrq1_m.clone(), true); // [N, 384, L]
            let zr = [(c.zr1.clone(), 0), (m1, 0)];
            let rh = kernels::gru_gate(&zr, None, h0f.clone());
            let q = mmc(self.q1_h.clone(), map(rh), true, gemm, implicit);
            // q + c.q1 + m1 from channel 256
            let h1 = kernels::gru_gate(&zr, Some(((q, 0), Some(c.q1.clone()), [(QFrom::Extra, 0), (QFrom::Zr(1), 256)])), h0f);
            let m2 = mmc_motion(self.zrq2_m.clone(), false);
            let zr = [(mmc(self.zr2_h.clone(), map(h1.clone()), false, gemm, implicit), 0), (c.zrq2.clone(), 0), (m2, 0)];
            let rh = kernels::gru_gate(&zr, None, h1.clone());
            let q = mmc(self.q2_h.clone(), map(rh), false, gemm, implicit);
            // q + c.zrq2 and m2 from channel 256
            return map(kernels::gru_gate(&zr, Some(((q, 0), None, [(QFrom::Zr(1), 256), (QFrom::Zr(2), 256)])), h1));
        }
        let one_minus = |z: Tensor<3>| z.neg().add_scalar(1.0);
        // Horizontal
        let m1 = mmc_motion(self.zrq1_m.clone(), true); // [N, 384, L]
        let zr = c.zr1.clone() + m1.clone().narrow(1, 0, 256);
        let z = sigmoid(zr.clone().narrow(1, 0, 128));
        let r = sigmoid(zr.narrow(1, 128, 128));
        let h0f = flat(h0.clone());
        let q = tanh(mmc(self.q1_h.clone(), map(r * h0f.clone()), true, gemm, implicit) + c.q1.clone() + m1.narrow(1, 256, 128));
        let h1 = one_minus(z.clone()) * h0f + z * q;
        // Vertical
        let m2 = mmc_motion(self.zrq2_m.clone(), false);
        let zr = mmc(self.zr2_h.clone(), map(h1.clone()), false, gemm, implicit) + c.zrq2.clone().narrow(1, 0, 256) + m2.clone().narrow(1, 0, 256);
        let z = sigmoid(zr.clone().narrow(1, 0, 128));
        let r = sigmoid(zr.narrow(1, 128, 128));
        let q = tanh(mmc(self.q2_h.clone(), map(r * h1.clone()), false, gemm, implicit) + c.zrq2.clone().narrow(1, 256, 128) + m2.narrow(1, 256, 128));
        map(one_minus(z.clone()) * h1 + z * q)
    }
}

/// The flow at the next finer level with plain operations (kernels::flow_double): 2 * bilinear(flow), corners aligned, as
/// one matmul per axis with the interpolation weights (exact; not burn's interpolation, see flow_double_kernel).
/// flow [B, C, H, W] -> [B, C, 2H, 2W]
pub fn flow_double(flow: Tensor<4>) -> Tensor<4> {
    let [b, c, h, w] = flow.dims();
    let device = flow.device();
    // [n, 2n]: input index -> output index weights, source position o * (n - 1) / (2n - 1)
    let weights = |n: usize, scale: f32| {
        let on = 2 * n;
        let mut m = vec![0f32; n * on];
        for o in 0..on {
            let num = o * (n - 1);
            let i0 = num / (on - 1);
            let t = (num - i0 * (on - 1)) as f32 / (on - 1) as f32;
            m[i0 * on + o] += scale * (1.0 - t);
            if t > 0.0 { m[(i0 + 1) * on + o] += scale * t; }
        }
        Tensor::<2>::from_data(TensorData::new(m, [n, on]), &device)
    };
    let rows = flow.reshape([b * c * h, w]).matmul(weights(w, 2.0)).reshape([b * c, h, 2 * w]);
    let cols = rows.swap_dims(1, 2).reshape([b * c * 2 * w, h]).matmul(weights(h, 1.0)).reshape([b * c, 2 * w, 2 * h]);
    cols.swap_dims(1, 2).reshape([b, c, 2 * h, 2 * w])
}

/// upsample_flow_with_mask (convex upsampling, factor 4): flow [B, 2, H, W], mask [B, 9*16, H, W] -> [B, 2, 4H, 4W]
pub fn convex_upsample(flow: Tensor<4>, mask: Tensor<4>) -> Tensor<4> {
    let [b, _, h, w] = flow.dims();
    let k = UPSAMPLE;
    // mask.view(b, 1, 9, k, k, h, w), softmax over the 9
    let mask = softmax(mask.reshape([b, 9, k * k, h * w]), 1);
    // F.unfold(k * flow, 3x3, padding 1): per channel, the 3x3 window row-major, zeros outside
    let mut up: Option<Tensor<4>> = None; // [B, 2, k*k, h*w]
    for (i, nb) in shifted(flow.mul_scalar(k as f32), 1).into_iter().enumerate() {
        let term = mask.clone().narrow(1, i, 1) * nb.reshape([b, 2, 1, h * w]);
        up = Some(match up { Some(u) => u + term, None => term });
    }
    // [B, 2, ky, kx, h, w] -> [B, 2, h, ky, w, kx]
    up.expect("nine neighbours")
        .reshape([b * 2, k, k, h, w])
        .permute([0, 3, 1, 4, 2])
        .reshape([b, 2, h * k, w * k])
}

// ---------------------------------------------------------------- The model

#[derive(Module, Debug)]
pub struct Gmflow {
    pub backbone: CnnEncoder,
    pub transformer: FeatureTransformer,
    pub feature_flow_attn: SelfAttnPropagation,
    pub refine_proj: Conv2d,
    pub refine: BasicUpdateBlock,
}

/// The size-dependent constants of one input size
pub struct Statics {
    pub eighth: Windows,
    pub quarter: Windows,
    /// ImageNet mean and std, [1, 3, 1, 1]
    pub mean: Tensor<4>,
    pub std: Tensor<4>,
    pub ops: Ops,
}

impl Statics {
    pub fn new(h: usize, w: usize, device: &Device) -> Self {
        assert!(h % 32 == 0 && w % 32 == 0, "gmflow needs a size divisible by 32 (the 8 window splits at 1/4), got {w}x{h}");
        let c3 = |v: [f32; 3]| Tensor::<4>::from_data(TensorData::new(v.to_vec(), [1, 3, 1, 1]), device);
        Self {
            eighth: Windows::new(h / 8, w / 8, 2, device),
            quarter: Windows::new(h / 4, w / 4, 8, device),
            mean: c3([0.485, 0.456, 0.406]),
            std: c3([0.229, 0.224, 0.225]),
            ops: Ops { kernels: true, conv_im2col: false, gru_factored: false, own_gemm: false, gemm_linear: false, fused: false, implicit: false, coop: false },
        }
    }
}

/// Intermediates of one pass, named as ci-gpu/refs_native.py names those of unimatch, for checking the port stage by stage
/// or, with `timed`, when each stage was done (the device synced at each point; nothing is kept); with `echo` as well,
/// "STAGE-MARK <name>" printed at each point, after the sync: in a log of cubecl's profile (each kernel printed as it is
/// timed), the kernels of a stage are those just before its mark
#[derive(Default)]
pub struct Trace {
    pub tensors: Vec<(String, Tensor<4>)>,
    pub timed: bool,
    pub echo: bool,
    pub marks: Vec<(String, std::time::Instant)>,
}

fn put(trace: &mut Option<&mut Trace>, name: &str, t: &Tensor<4>) {
    let Some(tr) = trace.as_deref_mut() else { return };
    if tr.timed {
        t.device().sync().expect("gmflow: GPU sync");
        tr.marks.push((name.to_owned(), std::time::Instant::now()));
        if tr.echo {
            println!("STAGE-MARK {name}");
        }
    } else {
        tr.tensors.push((name.to_owned(), t.clone()));
    }
}

/// The CNN features of images, [N, 128, H/4, W/4] and [N, 128, H/8, W/8]: all a pass needs of a frame on its own (the CNN
/// normalises each image by itself), so a frame in two pairs (the tracker: with the previous and with the next frame) goes
/// through the CNN once
#[derive(Clone)]
pub struct Features {
    pub quarter: Tensor<4>,
    pub eighth: Tensor<4>,
}

impl Features {
    fn split(self, b: usize) -> (Features, Features) {
        let (q, e) = (self.quarter, self.eighth);
        (Features { quarter: q.clone().narrow(0, 0, b), eighth: e.clone().narrow(0, 0, b) }, Features { quarter: q.narrow(0, b, b), eighth: e.narrow(0, b, b) })
    }
}

impl Gmflow {
    pub fn new(device: &Device) -> Self {
        Self {
            backbone: CnnEncoder::new(device),
            transformer: FeatureTransformer::new(device),
            feature_flow_attn: SelfAttnPropagation::new(device),
            refine_proj: Conv2dConfig::new([128, 256], [1, 1]).with_bias(true).init(device),
            refine: BasicUpdateBlock::new(device),
        }
    }

    /// The CNN features of raw 0-255 images [N, 3, H, W]
    pub fn features(&self, img: Tensor<4>, st: &Statics) -> Features {
        let x = (img.div_scalar(255.0) - st.mean.clone()) / st.std.clone();
        let (quarter, eighth) = self.backbone.forward(x, st.ops);
        Features { quarter, eighth }
    }

    /// Raw 0-255 images [B, 3, H, W] (B pairs) -> flow img0 -> img1 [B, 2, H, W], the forward of unimatch for the flow task
    pub fn forward(&self, img0: Tensor<4>, img1: Tensor<4>, st: &Statics) -> Tensor<4> {
        self.forward_traced(img0, img1, st, None)
    }

    pub fn forward_traced(&self, img0: Tensor<4>, img1: Tensor<4>, st: &Statics, mut trace: Option<&mut Trace>) -> Tensor<4> {
        let b = img0.dims()[0];
        let both = self.features(Tensor::cat(vec![img0, img1], 0), st);
        put(&mut trace, "backbone0", &both.quarter);
        put(&mut trace, "backbone1", &both.eighth);
        let (a, bb) = both.split(b);
        self.flow(&a, &bb, false, st, trace)
    }

    /// The flow from the frames of `a` to those of `b` [B, 2, H, W]; with `bidirectional`, the flows both ways in one pass
    /// [2B, 2, H, W] (a -> b, then b -> a), as unimatch computes them with pred_bidir_flow: the transformer at 1/8 and its
    /// correlation are shared (the correlation the other way is its transpose), the rest runs on both directions batched
    pub fn flow(&self, a: &Features, b: &Features, bidirectional: bool, st: &Statics, mut trace: Option<&mut Trace>) -> Tensor<4> {
        // The buffers are sized for one pair (bidirectional: the projections at 1/4 are 113 MiB, the 1/8 correlation 66 MB)
        assert_eq!(a.eighth.dims()[0], 1, "gmflow: one pair at a time");
        let ops = st.ops;
        // 1/8: global matching, global propagation
        let pos = st.eighth.position.clone();
        let (f0, f1) = (a.eighth.clone() + pos.clone(), b.eighth.clone() + pos);
        put(&mut trace, "pos_f0_0", &f0);
        put(&mut trace, "pos_f1_0", &f1);
        let (f0, f1) = self.transformer.forward(f0, f1, &st.eighth, ops.linear_gemm(), ops.fused);
        put(&mut trace, "transformer_f0_0", &f0);
        put(&mut trace, "transformer_f1_0", &f1);
        let flow = global_correlation_softmax(f0.clone(), f1.clone(), bidirectional, ops.linear_gemm(), ops.fused);
        put(&mut trace, "global_corr_flow0", &flow);
        let feature0 = if bidirectional { Tensor::cat(vec![f0, f1], 0) } else { f0 };
        let flow = self.feature_flow_attn.global(feature0, flow, ops.linear_gemm(), ops.fused);
        put(&mut trace, "propagated0", &flow);

        // 1/4: the flow doubled in size, feature 1 warped by it, local matching of what remains, local propagation
        let (f0_ori, f1_ori) = if bidirectional {
            (Tensor::cat(vec![a.quarter.clone(), b.quarter.clone()], 0), Tensor::cat(vec![b.quarter.clone(), a.quarter.clone()], 0))
        } else {
            (a.quarter.clone(), b.quarter.clone())
        };
        let flow = if st.ops.kernels { kernels::flow_double(flow) } else { flow_double(flow) };
        put(&mut trace, "warp_flow0", &flow);
        let f1w = flow_warp(f1_ori.clone(), flow.clone());
        put(&mut trace, "warped_feature1_0", &f1w);
        let pos = st.quarter.position.clone();
        let (f0, f1) = (f0_ori.clone() + pos.clone(), f1w + pos);
        put(&mut trace, "pos_f0_1", &f0);
        put(&mut trace, "pos_f1_1", &f1);
        let (f0, f1) = self.transformer.forward(f0, f1, &st.quarter, ops.linear_gemm(), ops.fused);
        put(&mut trace, "transformer_f0_1", &f0);
        put(&mut trace, "transformer_f1_1", &f1);
        let flow_pred = if ops.kernels { kernels::local_corr_softmax(f0.clone(), f1) } else { local_correlation_softmax(f0.clone(), f1, LOCAL_RADIUS) };
        put(&mut trace, "local_corr_flow0", &flow_pred);
        let flow = flow + flow_pred;
        let mut flow = self.feature_flow_attn.local(f0.clone(), flow, 1, ops.kernels, ops.linear_gemm());
        put(&mut trace, "propagated1", &flow);

        // Refinement: the context comes from feature 0 of the transformer and is the same in every iteration (unimatch
        // computes it again each time), the correlation from the CNN features
        // A 1x1 convolution: burn's but with `coop`
        let proj = conv_fwd(&self.refine_proj, f0, ops);
        put(&mut trace, "refine_proj0", &proj);
        let net0 = tanh(proj.clone().narrow(1, 0, 128));
        let inp = relu(proj.narrow(1, 128, 128));
        let mut net = net0.clone();
        let factored = ops.gru_factored.then(|| {
            let g = self.refine.factor_gru();
            let c = g.constants(net0.clone(), inp.clone(), ops.conv_gemm(), ops.implicit);
            (g, c)
        });
        for i in 0..REFINE_ITERATIONS {
            let corr = if ops.kernels {
                kernels::corr_with_flow(f0_ori.clone(), f1_ori.clone(), flow.clone())
            } else {
                local_correlation_with_flow(f0_ori.clone(), f1_ori.clone(), flow.clone(), LOCAL_RADIUS)
            };
            put(&mut trace, &format!("corr_with_flow{i}"), &corr);
            let (n, delta) = self.refine.forward(net0.clone(), inp.clone(), corr, flow.clone(), ops, factored.as_ref().map(|(g, c)| (g, c)));
            put(&mut trace, &format!("refine_net{i}"), &n);
            put(&mut trace, &format!("refine_delta{i}"), &delta);
            net = n;
            flow = flow + delta;
        }
        let mask = self.refine.up_mask(net, ops);
        let up = if ops.kernels { kernels::convex_upsample(flow, mask) } else { convex_upsample(flow, mask) };
        put(&mut trace, "upsampled0", &up);
        up
    }
}
