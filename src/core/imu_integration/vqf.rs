// Ported from https://github.com/dlaidig/vqf
// Based on: D. Laidig, T. Seel. "VQF: Highly Accurate IMU Orientation Estimation with Bias Estimation and Magnetic Disturbance Rejection."
// https://arxiv.org/abs/2203.17024

#[allow(dead_code)]
const EPS: f64 = f64::EPSILON;
const NAN: f64 = f64::NAN;
const DEG2RAD: f64 = std::f64::consts::PI/180.0;
const M_PI: f64 = std::f64::consts::PI;
fn square(x: f64) -> f64 { x*x }

#[derive(Clone)]
pub struct VQFParams {
    pub tau_acc: f64,
    pub tau_mag: f64,
    pub motion_bias_est_enabled: bool,
    pub rest_bias_est_enabled: bool,
    pub mag_dist_rejection_enabled: bool,
    pub bias_sigma_init: f64,
    pub bias_forgetting_time: f64,
    pub bias_clip: f64,
    pub bias_sigma_motion: f64,
    pub bias_vertical_forgetting_factor: f64,
    pub bias_sigma_rest: f64,
    pub rest_min_t: f64,
    pub rest_filter_tau: f64,
    pub rest_th_gyr: f64,
    pub rest_th_acc: f64,
    pub mag_current_tau: f64,
    pub mag_ref_tau: f64,
    pub mag_norm_th: f64,
    pub mag_dip_th: f64,
    pub mag_new_time: f64,
    pub mag_new_first_time: f64,
    pub mag_new_min_gyr: f64,
    pub mag_min_undisturbed_time: f64,
    pub mag_max_rejection_time: f64,
    pub mag_rejection_factor: f64,
}

impl Default for VQFParams {
    fn default() -> Self {
        Self {
            tau_acc: 3.0,
            tau_mag: 9.0,
            motion_bias_est_enabled: true,
            rest_bias_est_enabled: true,
            mag_dist_rejection_enabled: true,
            bias_sigma_init: 0.5,
            bias_forgetting_time: 100.0,
            bias_clip: 2.0,
            bias_sigma_motion: 0.1,
            bias_vertical_forgetting_factor: 0.0001,
            bias_sigma_rest: 0.03,
            rest_min_t: 1.5,
            rest_filter_tau: 0.5,
            rest_th_gyr: 2.0,
            rest_th_acc: 0.5,
            mag_current_tau: 0.05,
            mag_ref_tau: 20.0,
            mag_norm_th: 0.1,
            mag_dip_th: 10.0,
            mag_new_time: 20.0,
            mag_new_first_time: 5.0,
            mag_new_min_gyr: 20.0,
            mag_min_undisturbed_time: 0.5,
            mag_max_rejection_time: 60.0,
            mag_rejection_factor: 2.0,
        }
    }
}

#[allow(dead_code)]
#[derive(Default, Clone)]
pub struct VQFState {
    pub gyr_quat: [f64; 4],
    pub acc_quat: [f64; 4],
    pub delta: f64,
    pub rest_detected: bool,
    pub mag_dist_detected: bool,
    pub last_acc_lp: [f64; 3],
    pub acc_lp_state: [f64; 6],
    pub last_acc_corr_angular_rate: f64,
    pub k_mag_init: f64,
    pub last_mag_dis_angle: f64,
    pub last_mag_corr_angular_rate: f64,
    pub bias: [f64; 3],
    pub bias_p: [f64; 9],
    pub motion_bias_est_r_lp_state: [f64; 18],
    pub motion_bias_est_bias_lp_state: [f64; 4],
    pub rest_last_squared_deviations: [f64; 2],
    pub rest_t: f64,
    pub rest_last_gyr_lp: [f64; 3],
    pub rest_gyr_lp_state: [f64; 6],
    pub rest_last_acc_lp: [f64; 3],
    pub rest_acc_lp_state: [f64; 6],
    pub mag_ref_norm: f64,
    pub mag_ref_dip: f64,
    pub mag_undisturbed_t: f64,
    pub mag_reject_t: f64,
    pub mag_candidate_norm: f64,
    pub mag_candidate_dip: f64,
    pub mag_candidate_t: f64,
    pub mag_norm_dip: [f64; 2],
    pub mag_norm_dip_lp_state: [f64; 4],
}

#[allow(dead_code)]
#[derive(Default)]
pub struct VQFCoefficients {
    pub gyr_ts: f64,
    pub acc_ts: f64,
    pub mag_ts: f64,
    pub acc_lp_b: [f64; 3],
    pub acc_lp_a: [f64; 2],
    pub k_mag: f64,
    pub bias_p0: f64,
    pub bias_v: f64,
    pub bias_motion_w: f64,
    pub bias_vertical_w: f64,
    pub bias_rest_w: f64,
    pub rest_gyr_lp_b: [f64; 3],
    pub rest_gyr_lp_a: [f64; 2],
    pub rest_acc_lp_b: [f64; 3],
    pub rest_acc_lp_a: [f64; 2],
    pub k_mag_ref: f64,
    pub mag_norm_dip_lp_b: [f64; 3],
    pub mag_norm_dip_lp_a: [f64; 2]
}

#[allow(dead_code)]
pub struct VQF {
    pub params: VQFParams,
    pub state: VQFState,
    pub coeffs: VQFCoefficients,
}

#[allow(dead_code)]
impl VQF {
    pub fn vqf(params: Option<VQFParams>, gyr_ts: f64, acc_ts: f64, mag_ts: f64) -> Self {
        let mut coeffs = VQFCoefficients::default();
        coeffs.gyr_ts = gyr_ts;
        coeffs.acc_ts = if acc_ts > 0.0 { acc_ts } else { gyr_ts };
        coeffs.mag_ts = if mag_ts > 0.0 { mag_ts } else { gyr_ts };
        let mut out = Self { params: params.unwrap_or(VQFParams::default()), state: VQFState::default(), coeffs: coeffs };
        out.setup();
        out
    }

    // Performs gyroscope update step.
    pub fn update_gyr(&mut self, gyr: &[f64]) {

        // rest detection
        if self.params.rest_bias_est_enabled || self.params.mag_dist_rejection_enabled {
            VQF::filter_vec(&gyr, 3, self.params.rest_filter_tau, self.coeffs.gyr_ts, self.coeffs.rest_gyr_lp_b, self.coeffs.rest_gyr_lp_a,
                    &mut self.state.rest_gyr_lp_state, &mut self.state.rest_last_gyr_lp);

                    self.state.rest_last_squared_deviations[0] = square(gyr[0] - self.state.rest_last_gyr_lp[0])
                    + square(gyr[1] - self.state.rest_last_gyr_lp[1]) + square(gyr[2] - self.state.rest_last_gyr_lp[2]);

            let bias_clip = self.params.bias_clip * DEG2RAD;

            if self.state.rest_last_squared_deviations[0] >= square(self.params.rest_th_gyr*DEG2RAD)
                    || self.state.rest_last_gyr_lp[0].abs() > bias_clip || self.state.rest_last_gyr_lp[1].abs() > bias_clip
                    || self.state.rest_last_gyr_lp[2].abs() > bias_clip {
                self.state.rest_t = 0.0;
                self.state.rest_detected = false;
            }
        }

        // remove estimated gyro bias
        let gyr_no_bias: [f64; 3] = [gyr[0]-self.state.bias[0], gyr[1]-self.state.bias[1], gyr[2]-self.state.bias[2]];

        // gyroscope prediction step
        let gyr_norm = VQF::norm(&gyr_no_bias, 3);
        let angle = gyr_norm * self.coeffs.gyr_ts;
        if gyr_norm > EPS {
            let c = (angle/2.0).cos();
            let s = (angle/2.0).sin()/gyr_norm;
            let gyr_step_quat: [f64; 4] = [c, s*gyr_no_bias[0], s*gyr_no_bias[1], s*gyr_no_bias[2]];
            self.state.gyr_quat = VQF::quat_multiply(&self.state.gyr_quat, &gyr_step_quat);
            VQF::normalize(&mut self.state.gyr_quat, 4);
        }
    }

    // Performs accelerometer update step.
    pub fn update_acc(&mut self, acc: &[f64]) {
        // ignore [0 0 0] samples
        if acc[0].abs() == 0.0 && acc[1].abs() == 0.0 && acc[2].abs() == 0.0 {
            return;
        }

        // rest detection
        if self.params.rest_bias_est_enabled {
            VQF::filter_vec(&acc, 3, self.params.rest_filter_tau, self.coeffs.acc_ts, self.coeffs.rest_acc_lp_b, self.coeffs.rest_acc_lp_a,
                    &mut self.state.rest_acc_lp_state, &mut self.state.rest_last_acc_lp);

            self.state.rest_last_squared_deviations[1] = square(acc[0] - self.state.rest_last_acc_lp[0])
                    + square(acc[1] - self.state.rest_last_acc_lp[1]) + square(acc[2] - self.state.rest_last_acc_lp[2]);

            if self.state.rest_last_squared_deviations[1] >= square(self.params.rest_th_acc) {
                self.state.rest_t = 0.0;
                self.state.rest_detected = false;
            } else {
                self.state.rest_t += self.coeffs.acc_ts;
                if self.state.rest_t >= self.params.rest_min_t {
                    self.state.rest_detected = true;
                }
            }
        }

        // filter acc in inertial frame
        let mut acc_earth = VQF::quat_rotate(&self.state.gyr_quat, acc);
        VQF::filter_vec(&acc_earth, 3, self.params.tau_acc, self.coeffs.acc_ts, self.coeffs.acc_lp_b, self.coeffs.acc_lp_a, &mut self.state.acc_lp_state, &mut self.state.last_acc_lp);

        // transform to 6D earth frame and normalize
        acc_earth = VQF::quat_rotate(&self.state.acc_quat, &self.state.last_acc_lp);
        VQF::normalize(&mut acc_earth, 3);

        // inclination correction
        let mut acc_corr_quat = [0f64; 4];
        let q_w = ((acc_earth[2]+1.0)/2.0).sqrt();
        if q_w > 1e-6 {
            acc_corr_quat[0] = q_w;
            acc_corr_quat[1] = 0.5*acc_earth[1]/q_w;
            acc_corr_quat[2] = -0.5*acc_earth[0]/q_w;
            acc_corr_quat[3] = 0.0;
        } else {
            // to avoid numeric issues when acc is close to [0 0 -1], i.e. the correction step is close (<= 0.00011°) to 180°:
            acc_corr_quat[0] = 0.0;
            acc_corr_quat[1] = 1.0;
            acc_corr_quat[2] = 0.0;
            acc_corr_quat[3] = 0.0;
        }
        self.state.acc_quat = VQF::quat_multiply(&acc_corr_quat, &self.state.acc_quat);
        VQF::normalize(&mut self.state.acc_quat, 4);

        // calculate correction angular rate to facilitate debugging
        self.state.last_acc_corr_angular_rate = (acc_earth[2]).acos()/self.coeffs.acc_ts;

        // bias estimation

        if self.params.motion_bias_est_enabled || self.params.rest_bias_est_enabled {
            let bias_clip = self.params.bias_clip*DEG2RAD;

            let mut r = [NAN; 9];
            let mut bias_lp = [NAN; 2];

            // get rotation matrix corresponding to accGyrQuat
            let acc_gyr_quat = self.get_quat6d();
            r[0] = 1.0 - 2.0*square(acc_gyr_quat[2]) - 2.0*square(acc_gyr_quat[3]); // r11
            r[1] = 2.0*(acc_gyr_quat[2]*acc_gyr_quat[1] - acc_gyr_quat[0]*acc_gyr_quat[3]); // r12
            r[2] = 2.0*(acc_gyr_quat[0]*acc_gyr_quat[2] + acc_gyr_quat[3]*acc_gyr_quat[1]); // r13
            r[3] = 2.0*(acc_gyr_quat[0]*acc_gyr_quat[3] + acc_gyr_quat[2]*acc_gyr_quat[1]); // r21
            r[4] = 1.0 - 2.0*square(acc_gyr_quat[1]) - 2.0*square(acc_gyr_quat[3]); // r22
            r[5] = 2.0*(acc_gyr_quat[2]*acc_gyr_quat[3] - acc_gyr_quat[1]*acc_gyr_quat[0]); // r23
            r[6] = 2.0*(acc_gyr_quat[3]*acc_gyr_quat[1] - acc_gyr_quat[0]*acc_gyr_quat[2]); // r31
            r[7] = 2.0*(acc_gyr_quat[0]*acc_gyr_quat[1] + acc_gyr_quat[3]*acc_gyr_quat[2]); // r32
            r[8] = 1.0 - 2.0*square(acc_gyr_quat[1]) - 2.0*square(acc_gyr_quat[2]); // r33

            // calculate R*b_hat (only the x and y component, as z is not needed)
            bias_lp[0] = r[0]*self.state.bias[0] + r[1]*self.state.bias[1] + r[2]*self.state.bias[2];
            bias_lp[1] = r[3]*self.state.bias[0] + r[4]*self.state.bias[1] + r[5]*self.state.bias[2];

            // low-pass filter R and R*b_hat
            VQF::filter_vec(&(r.clone()), 9, self.params.tau_acc, self.coeffs.acc_ts, self.coeffs.acc_lp_b, self.coeffs.acc_lp_a, &mut self.state.motion_bias_est_r_lp_state, &mut r);
            VQF::filter_vec(&(bias_lp.clone()), 2, self.params.tau_acc, self.coeffs.acc_ts, self.coeffs.acc_lp_b, self.coeffs.acc_lp_a, &mut self.state.motion_bias_est_bias_lp_state,
                    &mut bias_lp);

            // set measurement error and covariance for the respective Kalman filter update
            let mut w = [0f64; 3];
            let mut e = [0f64; 3];
            if self.state.rest_detected && self.params.rest_bias_est_enabled {
                e[0] = self.state.rest_last_gyr_lp[0] - self.state.bias[0];
                e[1] = self.state.rest_last_gyr_lp[1] - self.state.bias[1];
                e[2] = self.state.rest_last_gyr_lp[2] - self.state.bias[2];
                r = VQF::matrix3_set_to_scaled_identity(1.0);
                w = [self.coeffs.bias_rest_w; 3];
            } else if self.params.motion_bias_est_enabled {
                e[0] = -acc_earth[1]/self.coeffs.acc_ts + bias_lp[0] - r[0]*self.state.bias[0] - r[1]*self.state.bias[1] - r[2]*self.state.bias[2];
                e[1] = acc_earth[0]/self.coeffs.acc_ts + bias_lp[1] - r[3]*self.state.bias[0] - r[4]*self.state.bias[1] - r[5]*self.state.bias[2];
                e[2] = - r[6]*self.state.bias[0] - r[7]*self.state.bias[1] - r[8]*self.state.bias[2];
                w[0] = self.coeffs.bias_motion_w;
                w[1] = self.coeffs.bias_motion_w;
                w[2] = self.coeffs.bias_vertical_w;
            } else {
                w = [-1.0; 3]; // disable update
            }

            // Kalman filter update
            // step 1: P = P + V (also increase covariance if there is no measurement update!)
            if self.state.bias_p[0] < self.coeffs.bias_p0 {
                self.state.bias_p[0] += self.coeffs.bias_v;
            }
            if self.state.bias_p[4] < self.coeffs.bias_p0 {
                self.state.bias_p[4] += self.coeffs.bias_v;
            }
            if self.state.bias_p[8] < self.coeffs.bias_p0 {
                self.state.bias_p[8] += self.coeffs.bias_v;
            }
            if w[0] >= 0.0 {
                // clip disagreement to -2..2 °/s
                // (this also effectively limits the harm done by the first inclination correction step)
                VQF::clip(&mut e, 3, -bias_clip, bias_clip);

                // step 2: K = P R^T inv(W + R P R^T)
                let mut k = VQF::matrix3_multiply_tps_second(&self.state.bias_p, &r); // k = p r^t
                k = VQF::matrix3_multiply(&r, &k); // k = r p r^t
                k[0] += w[0];
                k[4] += w[1];
                k[8] += w[2]; // k = w + r p r^t
                k = VQF::matrix3_inv(&k); // k = inv(w + r p r^t)
                k = VQF::matrix3_multiply_tps_first(&r, &k); // k = r^t inv(w + r p r^t)
                k = VQF::matrix3_multiply(&self.state.bias_p, &k); // k = p r^t inv(w + r p r^t)

                // step 3: bias = bias + K (y - R bias) = bias + K e
                self.state.bias[0] += k[0]*e[0] + k[1]*e[1] + k[2]*e[2];
                self.state.bias[1] += k[3]*e[0] + k[4]*e[1] + k[5]*e[2];
                self.state.bias[2] += k[6]*e[0] + k[7]*e[1] + k[8]*e[2];

                // step 4: P = P - K R P
                k = VQF::matrix3_multiply(&k, &r); // k = k r
                k = VQF::matrix3_multiply(&k, &self.state.bias_p); // K = K R P
                for i in 0..9 {
                    self.state.bias_p[i] -= k[i];
                }

                // clip bias estimate to -2..2 °/s
                VQF::clip(&mut self.state.bias, 3, -bias_clip, bias_clip);
            }
        }
    }

    // Performs magnetometer update step.
    pub fn update_mag(&mut self, mag: &[f64]) {
        // ignore [0 0 0] samples
        if mag[0].abs() == 0.0 && mag[1].abs() == 0.0 && mag[2].abs() == 0.0 {
            return;
        }

        // bring magnetometer measurement into 6D earth frame
        let acc_gyr_quat = self.get_quat6d();
        let mag_earth = VQF::quat_rotate(&acc_gyr_quat, mag);

        if self.params.mag_dist_rejection_enabled {
            self.state.mag_norm_dip[0] = VQF::norm(&mag_earth, 3);
            self.state.mag_norm_dip[1] = -(mag_earth[2]/self.state.mag_norm_dip[0]).asin();

            if self.params.mag_current_tau > 0.0 {
                VQF::filter_vec(&(self.state.mag_norm_dip.clone()), 2, self.params.mag_current_tau, self.coeffs.mag_ts, self.coeffs.mag_norm_dip_lp_b,
                        self.coeffs.mag_norm_dip_lp_a, &mut self.state.mag_norm_dip_lp_state, &mut self.state.mag_norm_dip);
            }

            // magnetic disturbance detection
            if (self.state.mag_norm_dip[0] - self.state.mag_ref_norm).abs() < self.params.mag_norm_th*self.state.mag_ref_norm
                    && (self.state.mag_norm_dip[1] - self.state.mag_ref_dip).abs() < self.params.mag_dip_th*DEG2RAD {
                self.state.mag_undisturbed_t += self.coeffs.mag_ts;
                if self.state.mag_undisturbed_t >= self.params.mag_min_undisturbed_time {
                    self.state.mag_dist_detected = false;
                    self.state.mag_ref_norm += self.coeffs.k_mag_ref*(self.state.mag_norm_dip[0] - self.state.mag_ref_norm);
                    self.state.mag_ref_dip += self.coeffs.k_mag_ref*(self.state.mag_norm_dip[1] - self.state.mag_ref_dip);
                }
            } else {
                self.state.mag_undisturbed_t = 0.0;
                self.state.mag_dist_detected = true;
            }

            // new magnetic field acceptance
            if (self.state.mag_norm_dip[0] - self.state.mag_candidate_norm).abs() < self.params.mag_norm_th*self.state.mag_candidate_norm
                    && (self.state.mag_norm_dip[1] - self.state.mag_candidate_dip).abs() < self.params.mag_dip_th*DEG2RAD {
                if VQF::norm(&self.state.rest_last_gyr_lp, 3) >= self.params.mag_new_min_gyr*DEG2RAD {
                    self.state.mag_candidate_t += self.coeffs.mag_ts;
                }
                self.state.mag_candidate_norm += self.coeffs.k_mag_ref*(self.state.mag_norm_dip[0] - self.state.mag_candidate_norm);
                self.state.mag_candidate_dip += self.coeffs.k_mag_ref*(self.state.mag_norm_dip[1] - self.state.mag_candidate_dip);

                if self.state.mag_dist_detected && (self.state.mag_candidate_t >= self.params.mag_new_time || (
                    self.state.mag_ref_norm == 0.0 && self.state.mag_candidate_t >= self.params.mag_new_first_time)) {
                    self.state.mag_ref_norm = self.state.mag_candidate_norm;
                    self.state.mag_ref_dip = self.state.mag_candidate_dip;
                    self.state.mag_dist_detected = false;
                    self.state.mag_undisturbed_t = self.params.mag_min_undisturbed_time;
                }
            } else {
                self.state.mag_candidate_t = 0.0;
                self.state.mag_candidate_norm = self.state.mag_norm_dip[0];
                self.state.mag_candidate_dip = self.state.mag_norm_dip[1];
            }
        }

        // calculate disagreement angle based on current magnetometer measurement
        self.state.last_mag_dis_angle = mag_earth[0].atan2(mag_earth[1]) - self.state.delta;

        // make sure the disagreement angle is in the range [-pi, pi]
        if self.state.last_mag_dis_angle > M_PI {
            self.state.last_mag_dis_angle -= M_PI*2.0;
        } else if self.state.last_mag_dis_angle < -M_PI {
            self.state.last_mag_dis_angle += 2.0*M_PI;
        }

        let mut k = self.coeffs.k_mag;

        if self.params.mag_dist_rejection_enabled {
            // magnetic disturbance rejection
            if self.state.mag_dist_detected {
                if self.state.mag_reject_t <= self.params.mag_max_rejection_time {
                    self.state.mag_reject_t += self.coeffs.mag_ts;
                    k = 0.0;
                } else {
                    k /= self.params.mag_rejection_factor;
                }
            } else {
                self.state.mag_reject_t = (self.state.mag_reject_t - self.params.mag_rejection_factor*self.coeffs.mag_ts).max(0.0);
            }
        }

        // ensure fast initial convergence
        if self.state.k_mag_init.abs() != 0.0 {
            // make sure that the gain k is at least 1/N, N=1,2,3,... in the first few samples
            if k < self.state.k_mag_init {
                k = self.state.k_mag_init;
            }

            // iterative expression to calculate 1/N
            self.state.k_mag_init = self.state.k_mag_init/(self.state.k_mag_init+1.0);

            // disable if t > tauMag
            if self.state.k_mag_init*self.params.tau_mag < self.coeffs.mag_ts {
                self.state.k_mag_init = 0.0;
            }
        }

        // first-order filter step
        self.state.delta += k*self.state.last_mag_dis_angle;
        // calculate correction angular rate to facilitate debugging
        self.state.last_mag_corr_angular_rate = k*self.state.last_mag_dis_angle/self.coeffs.mag_ts;

        // make sure delta is in the range [-pi, pi]
        if self.state.delta > M_PI {
            self.state.delta -= 2.0*M_PI;
        } else if self.state.delta < -M_PI {
            self.state.delta += 2.0*M_PI;
        }
    }

    // Performs filter update step for one sample (with magnetometer measurement).
    pub fn update(&mut self, gyr: &[f64], acc: &[f64], mag: Option<&[f64]>) {
        self.update_gyr(gyr);
        self.update_acc(acc);
        if mag.is_some() {
            self.update_mag(mag.unwrap());
        }
    }

    // Performs batch update for multiple samples at once.
    pub fn update_batch(&mut self, gyr: &[f64], acc: &[f64], mag: Option<&[f64]>, n: usize, mut out6d: Option<&mut Vec<f64>>, mut out9d: Option<&mut Vec<f64>>, mut out_delta: Option<&mut Vec<f64>>,
        mut out_bias: Option<&mut Vec<f64>>, mut out_bias_sigma: Option<&mut Vec<f64>>, mut out_rest: Option<&mut Vec<bool>>, mut out_mag_dist: Option<&mut Vec<bool>>) {

        for i in 0..n {
            let g = &gyr[3*i..3*i+3];
            let a = &acc[3*i..3*i+3];
            if let Some(mag) = mag {
                let m = &mag[3*i..3*i+3];
                self.update(g, a, Some(m));
            } else {
                self.update(g, a, None);
            }
            if let Some(ref mut out6d) = out6d {
                out6d.splice(4*i..4*i+4, self.get_quat6d().into_iter());
            }
            if let Some(ref mut out9d) = out9d {
                out9d.splice(4*i..4*i+4, self.get_quat9d().into_iter());
            }
            if let Some(ref mut out_delta) = out_delta {
                out_delta[i] = self.state.delta.clone();
            }
            if let Some(ref mut out_bias) = out_bias {
                out_bias.splice(3*i..3*i+3, self.state.bias.iter().cloned());
            }
            if let Some(ref mut out_bias_sigma) = out_bias_sigma {
                (_, out_bias_sigma[i]) = self.get_bias_estimate();
            }
            if let Some(ref mut out_rest) = out_rest {
                out_rest[i] = self.state.rest_detected.clone();
            }
            if let Some(ref mut out_mag_dist) = out_mag_dist {
                out_mag_dist[i] = self.state.mag_dist_detected.clone();
            }
        }
    }

    // Returns the angular velocity strapdown integration quaternion
    pub fn get_quat3d(&self) -> &[f64; 4] {
        &self.state.gyr_quat
    }

    // Returns the 6D (magnetometer-free) orientation quaternion
    pub fn get_quat6d(&self) -> [f64; 4] {
        VQF::quat_multiply(&self.state.acc_quat, &self.state.gyr_quat)
    }

    // Returns the 9D (with magnetometers) orientation quaternion
    pub fn get_quat9d(&self) -> [f64; 4] {
        VQF::quat_apply_delta(&VQF::quat_multiply(&self.state.acc_quat, &self.state.gyr_quat), self.state.delta)
    }

    // Returns the heading difference \f$\delta\f$ between \f$\mathcal{E}_i\f$ and \f$\mathcal{E}\f$.
    pub fn get_delta(&self) -> f64 {
        self.state.delta
    }

    // Returns the current gyroscope bias estimate and the uncertainty.
    pub fn get_bias_estimate(&self) -> ([f64; 3], f64) {

        // use largest absolute row sum as upper bound estimate for largest eigenvalue (Gershgorin circle theorem)
        // and clip output to biasSigmaInit
        let sum1 = self.state.bias_p[0].abs() + self.state.bias_p[1].abs() + self.state.bias_p[2].abs();
        let sum2 = self.state.bias_p[3].abs() + self.state.bias_p[4].abs() + self.state.bias_p[5].abs();
        let sum3 = self.state.bias_p[6].abs() + self.state.bias_p[7].abs() + self.state.bias_p[8].abs();
        let p = sum1.max(sum2).max(sum3).min(self.coeffs.bias_p0);

        (self.state.bias.clone(), p.sqrt()*M_PI/100.0/180.0)
    }

    // Sets the current gyroscope bias estimate and the uncertainty.
    pub fn set_bias_estimate(&mut self, bias: [f64; 3], sigma: Option<f64>) {
        let sigma_v = sigma.unwrap_or(-1.0);
        self.state.bias = bias.clone();
        if sigma_v > 0.0 {
            let p = square(sigma_v*(180.0*100.0/M_PI));
            self.state.bias_p = VQF::matrix3_set_to_scaled_identity(p);
        }
    }

    // Returns true if rest was detected.
    pub fn get_rest_detected(&self) -> bool {
        self.state.rest_detected
    }

    // Returns true if a disturbed magnetic field was detected.
    pub fn get_mag_dist_detected(&self) -> bool {
        self.state.mag_dist_detected
    }

    // Returns the relative deviations used in rest detection.
    pub fn get_relative_rest_deviations(&self) -> [f64; 2] {
        let mut out = [0.0; 2];
        out[0] = self.state.rest_last_squared_deviations[0].sqrt() / (self.params.rest_th_gyr*DEG2RAD);
        out[1] = self.state.rest_last_squared_deviations[1].sqrt() / self.params.rest_th_acc;
        out
    }

    // Returns the norm of the currently accepted magnetic field reference.
    pub fn get_mag_ref_norm(&self) -> f64 {
        self.state.mag_ref_norm
    }

    // Returns the dip angle of the currently accepted magnetic field reference.
    pub fn get_mag_ref_dip(&self) -> f64 {
        self.state.mag_ref_dip
    }

    // Overwrites the current magnetic field reference.
    pub fn set_mag_ref(&mut self, norm: f64, dip: f64) {
        self.state.mag_ref_norm = norm;
        self.state.mag_ref_dip = dip;
    }

    // Sets the time constant for accelerometer low-pass filtering.
    pub fn set_tau_acc(&mut self, tau_acc: f64) {
        if self.params.tau_acc == tau_acc {
            return;
        }
        self.params.tau_acc = tau_acc;
        let mut new_b = [0.0; 3];
        let mut new_a = [0.0; 2];

        VQF::filter_coeffs(self.params.tau_acc, self.coeffs.acc_ts, &mut new_b, &mut new_a);
        VQF::filter_adapt_state_for_coeff_change(&self.state.last_acc_lp, 3, self.coeffs.acc_lp_b,
                                                 self.coeffs.acc_lp_a, new_b, new_a, &mut self.state.acc_lp_state);

        // For R and biasLP, the last value is not saved in the state.
        // Since b0 is small (at reasonable settings), the last output is close to state[0].
        let mut r = [0.0; 9];
        for i in 0..9 {
            r[i] = self.state.motion_bias_est_r_lp_state[2*i];
        }
        VQF::filter_adapt_state_for_coeff_change(&r, 9, self.coeffs.acc_lp_b, self.coeffs.acc_lp_a,
                                                 new_b, new_a, &mut self.state.motion_bias_est_r_lp_state);
        let mut bias_lp = [0.0; 2];
        for i in 0..2 {
            bias_lp[i] = self.state.motion_bias_est_bias_lp_state[2*i];
        }
        VQF::filter_adapt_state_for_coeff_change(&bias_lp, 2, self.coeffs.acc_lp_b, self.coeffs.acc_lp_a, new_b,
                                                new_a, &mut self.state.motion_bias_est_bias_lp_state);

        self.coeffs.acc_lp_b = new_b;
        self.coeffs.acc_lp_a = new_a;
    }

    // Sets the time constant for the magnetometer update.
    pub fn set_tau_mag(&mut self, tau_mag: f64) {
        self.params.tau_mag = tau_mag;
        self.coeffs.k_mag = VQF::gain_from_tau(self.params.tau_mag, self.coeffs.mag_ts);
    }

    // Enables/disabled gyroscope bias estimation during motion.
    pub fn set_motion_bias_est_enabled(&mut self, enabled: bool) {
        if self.params.motion_bias_est_enabled == enabled {
            return;
        }
        self.params.motion_bias_est_enabled = enabled;
        self.state.motion_bias_est_r_lp_state = [NAN; 18];
        self.state.motion_bias_est_bias_lp_state = [NAN; 4];
    }

    // Enables/disables rest detection and bias estimation during rest.
    pub fn set_rest_bias_est_enabled(&mut self, enabled: bool) {
        if self.params.rest_bias_est_enabled == enabled {
            return;
        }
        self.params.rest_bias_est_enabled = enabled;
        self.state.rest_detected = false;
        self.state.rest_last_squared_deviations = [0.0; 2];
        self.state.rest_t = 0.0;
        self.state.rest_last_gyr_lp = [0.0; 3];
        self.state.rest_gyr_lp_state = [NAN; 6];
        self.state.rest_last_acc_lp = [0.0; 3];
        self.state.rest_acc_lp_state = [NAN; 6];
    }

    // Enables/disables magnetic disturbance detection and rejection.
    pub fn set_mag_dist_rejection_enabled(&mut self, enabled: bool) {
        if self.params.mag_dist_rejection_enabled == enabled {
            return;
        }
        self.params.mag_dist_rejection_enabled = enabled;
        self.state.mag_dist_detected = true;
        self.state.mag_ref_norm = 0.0;
        self.state.mag_ref_dip = 0.0;
        self.state.mag_undisturbed_t = 0.0;
        self.state.mag_reject_t = self.params.mag_max_rejection_time;
        self.state.mag_candidate_norm = -1.0;
        self.state.mag_candidate_dip = 0.0;
        self.state.mag_candidate_t = 0.0;
        self.state.mag_norm_dip_lp_state = [NAN; 4];
    }

    // Sets the current thresholds for rest detection.
    pub fn set_rest_detection_thresholds(&mut self, th_gyr: f64, th_acc: f64) {
        self.params.rest_th_gyr = th_gyr;
        self.params.rest_th_acc = th_acc;
    }

    // Returns the current parameters.
    pub fn get_params(&self) -> &VQFParams {
        &self.params
    }

    // Returns the coefficients used by the algorithm.
    pub fn get_coeffs(&self) -> &VQFCoefficients {
        &self.coeffs
    }

    // Returns the current state.
    pub fn get_state(&self) -> &VQFState {
        &self.state
    }

    // Overwrites the current state.
    pub fn set_state(&mut self, state: VQFState) {
        self.state = state;
    }

    // Resets the state to the default values at initialization.
    pub fn reset_state(&mut self) {
        VQF::quat_set_to_identity(&mut self.state.gyr_quat);
        VQF::quat_set_to_identity(&mut self.state.acc_quat);
        self.state.delta = 0.0;

        self.state.rest_detected = false;
        self.state.mag_dist_detected = true;

        self.state.last_acc_lp = [0.0; 3];
        self.state.acc_lp_state = [NAN; 6];
        self.state.last_acc_corr_angular_rate = 0.0;

        self.state.k_mag_init = 1.0;
        self.state.last_mag_dis_angle = 0.0;
        self.state.last_mag_corr_angular_rate = 0.0;

        self.state.bias = [0.0; 3];
        self.state.bias_p = VQF::matrix3_set_to_scaled_identity(self.coeffs.bias_p0);

        self.state.motion_bias_est_r_lp_state = [NAN; 18];
        self.state.motion_bias_est_bias_lp_state = [NAN; 4];

        self.state.rest_last_squared_deviations = [0.0; 2];
        self.state.rest_t = 0.0;
        self.state.rest_last_gyr_lp = [NAN; 3];
        self.state.rest_gyr_lp_state = [NAN; 6];
        self.state.rest_last_acc_lp = [0.0; 3];
        self.state.rest_acc_lp_state = [NAN; 6];

        self.state.mag_ref_norm = 0.0;
        self.state.mag_ref_dip = 0.0;
        self.state.mag_undisturbed_t = 0.0;
        self.state.mag_reject_t = self.params.mag_max_rejection_time;
        self.state.mag_candidate_norm = -1.0;
        self.state.mag_candidate_dip = 0.0;
        self.state.mag_candidate_t = 0.0;
        self.state.mag_norm_dip = [0.0; 2];
        self.state.mag_norm_dip_lp_state = [NAN; 4];
    }

    // Performs quaternion multiplication
    pub fn quat_multiply(q1: &[f64], q2: &[f64]) -> [f64; 4] {
        let w = q1[0] * q2[0] - q1[1] * q2[1] - q1[2] * q2[2] - q1[3] * q2[3];
        let x = q1[0] * q2[1] + q1[1] * q2[0] + q1[2] * q2[3] - q1[3] * q2[2];
        let y = q1[0] * q2[2] - q1[1] * q2[3] + q1[2] * q2[0] + q1[3] * q2[1];
        let z = q1[0] * q2[3] + q1[1] * q2[2] - q1[2] * q2[1] + q1[3] * q2[0];
        [w, x, y, z]
    }

    // Calculates the quaternion conjugate
    pub fn quat_conj(q: &[f64]) -> [f64; 4] {
        [q[0], -q[1], -q[2], -q[3]]
    }

    // Sets the output quaternion to the identity quaternion
    pub fn quat_set_to_identity(out: &mut [f64; 4]) {
        out[0] = 1.0;
        out[1] = 0.0;
        out[2] = 0.0;
        out[3] = 0.0;
    }

    // Applies a heading rotation by the angle delta (in rad) to a quaternion.
    pub fn quat_apply_delta(q: &[f64], delta: f64) -> [f64; 4] {
        // out = quatMultiply([cos(delta/2), 0, 0, sin(delta/2)], q)
        let c = (delta/2.0).cos();
        let s = (delta/2.0).sin();
        let w = c * q[0] - s * q[3];
        let x = c * q[1] - s * q[2];
        let y = c * q[2] + s * q[1];
        let z = c * q[3] + s * q[0];
        [w, x, y, z]
    }

    // Rotates a vector with a given quaternion.
    pub fn quat_rotate(q: &[f64], v: &[f64]) -> [f64; 3] {
        let x = (1.0 - 2.0*q[2]*q[2] - 2.0*q[3]*q[3])*v[0] + 2.0*v[1]*(q[2]*q[1] - q[0]*q[3]) + 2.0*v[2]*(q[0]*q[2] + q[3]*q[1]);
        let y = 2.0*v[0]*(q[0]*q[3] + q[2]*q[1]) + v[1]*(1.0 - 2.0*q[1]*q[1] - 2.0*q[3]*q[3]) + 2.0*v[2]*(q[2]*q[3] - q[1]*q[0]);
        let z = 2.0*v[0]*(q[3]*q[1] - q[0]*q[2]) + 2.0*v[1]*(q[0]*q[1] + q[3]*q[2]) + v[2]*(1.0 - 2.0*q[1]*q[1] - 2.0*q[2]*q[2]);
        [x, y, z]
    }

    // Calculates the Euclidean norm of a vector.
    pub fn norm(vec: &[f64], n: usize) -> f64 {
        let mut s = 0.0;
        for i in 0..n {
            s += vec[i]*vec[i];
        }
        s.sqrt()
    }

    // Normalizes a vector in-place.
    pub fn normalize(vec: &mut [f64], n: usize) {
        let l = VQF::norm(vec, n);
        if l < EPS {
            return;
        }
        for i in 0..n {
            vec[i] /= l;
        }
    }

    // Clips a vector in-place.
    pub fn clip(vec: &mut [f64], n: usize, min: f64, max: f64) {
        for i in 0..n {
            if vec[i] < min {
                vec[i] = min;
            } else if vec[i] > max {
                vec[i] = max;
            }
        }
    }

    // Calculates the gain for a first-order low-pass filter from the 1/e time constant.
    pub fn gain_from_tau(tau: f64, ts: f64) -> f64 {
        assert!(ts > 0.0);
        if tau < 0.0 {
            return 0.0; // k=0 for negative tau (disable update)
        } else if tau.abs() == 0.0 {
            return 1.0; // k=1 for tau=0
        } else {
            return 1.0 - (-ts/tau).exp();  // fc = 1/(2*pi*tau)
        }
    }

    // Calculates coefficients for a second-order Butterworth low-pass filter.
    pub fn filter_coeffs(tau: f64, ts: f64, out_b: &mut [f64; 3], out_a: &mut [f64; 2]) {
        assert!(tau > 0.0);
        assert!(ts > 0.0);
        const M_SQRT2: f64 = std::f64::consts::SQRT_2;
        // second order Butterworth filter based on https://stackoverflow.com/a/52764064
        let fc = (M_SQRT2 / (2.0*M_PI))/(tau); // time constant of dampened, non-oscillating part of step response
        let c = (M_PI*fc*ts).tan();
        let d = c*c + M_SQRT2*c + 1.0;
        let b0 = c*c/d;
        out_b[0] = b0;
        out_b[1] = 2.0*b0;
        out_b[2] = b0;
        // a0 = 1.0
        out_a[0] = 2.0*(c*c-1.0)/d; // a1
        out_a[1] = (1.0-M_SQRT2*c+c*c)/d; // a2
    }

    // Calculates the initial filter state for a given steady-state value.
    pub fn filter_initial_state(x0: f64, b: [f64; 3], a: [f64; 2], out: &mut [f64]) {
        out[0] = x0*(1.0 - b[0]);
        out[1] = x0*(b[2] - a[1]);
    }

    // Adjusts the filter state when changing coefficients.
    pub fn filter_adapt_state_for_coeff_change(last_y: &[f64], n: usize, b_old: [f64; 3],
                                          a_old: [f64; 2], b_new: [f64; 3],
                                          a_new: [f64; 2], state: &mut [f64]) {
        if state[0].is_nan() {
            return;
        }
        for i in 0..n {
            state[0+2*i] = state[0+2*i] + (b_old[0] - b_new[0])*last_y[i];
            state[1+2*i] = state[1+2*i] + (b_old[1] - b_new[1] - a_old[0] + a_new[0])*last_y[i];
        }
    }

    // Performs a filter step for a scalar value.
    pub fn filter_step(x: f64, b: [f64; 3], a: [f64; 2], state: &mut [f64]) -> f64 {
        let y = b[0]*x + state[0];
        state[0] = b[1]*x - a[0]*y + state[1];
        state[1] = b[2]*x - a[1]*y;
        y
    }

    // Performs filter step for vector-valued signal with averaging-based initialization.
    pub fn filter_vec(x: &[f64], n: usize, tau: f64, ts: f64, b: [f64; 3],
                     a: [f64; 2], state: &mut [f64], out: &mut [f64]) {

        assert!(n>=2);

        // to avoid depending on a single sample, average the first samples (for duration tau)
        // and then use this average to calculate the filter initial state
        if state[0].is_nan() { // initialization phase
            if state[1].is_nan() { // first sample
                state[1] = 0.0; // state[1] is used to store the sample count
                for i in 0..n {
                    state[2+i] = 0.0; // state[2+i] is used to store the sum
                }
            }
            state[1] += 1.0;
            for i in 0..n  {
                state[2+i] += x[i];
                out[i] = state[2+i]/state[1];
            }
            if state[1]*ts >= tau {
                for i in 0..n {
                    VQF::filter_initial_state(out[i], b, a, &mut state[2*i..2*i+2]);
                }
            }
            return;
        }

        for i in 0..n {
            out[i] = VQF::filter_step(x[i], b, a, &mut state[2*i..2*i+2]);
        }
    }

    pub fn matrix3_set_to_scaled_identity(scale: f64) -> [f64; 9] {
        [scale, 0.0, 0.0,
        0.0, scale, 0.0,
        0.0, 0.0, scale]
    }

    pub fn matrix3_multiply(in1: &[f64], in2: &[f64]) -> [f64; 9] {
        [in1[0]*in2[0] + in1[1]*in2[3] + in1[2]*in2[6],
        in1[0]*in2[1] + in1[1]*in2[4] + in1[2]*in2[7],
        in1[0]*in2[2] + in1[1]*in2[5] + in1[2]*in2[8],
        in1[3]*in2[0] + in1[4]*in2[3] + in1[5]*in2[6],
        in1[3]*in2[1] + in1[4]*in2[4] + in1[5]*in2[7],
        in1[3]*in2[2] + in1[4]*in2[5] + in1[5]*in2[8],
        in1[6]*in2[0] + in1[7]*in2[3] + in1[8]*in2[6],
        in1[6]*in2[1] + in1[7]*in2[4] + in1[8]*in2[7],
        in1[6]*in2[2] + in1[7]*in2[5] + in1[8]*in2[8]]
    }

    pub fn matrix3_multiply_tps_first(in1: &[f64], in2: &[f64]) -> [f64; 9] {
        [in1[0]*in2[0] + in1[3]*in2[3] + in1[6]*in2[6],
        in1[0]*in2[1] + in1[3]*in2[4] + in1[6]*in2[7],
        in1[0]*in2[2] + in1[3]*in2[5] + in1[6]*in2[8],
        in1[1]*in2[0] + in1[4]*in2[3] + in1[7]*in2[6],
        in1[1]*in2[1] + in1[4]*in2[4] + in1[7]*in2[7],
        in1[1]*in2[2] + in1[4]*in2[5] + in1[7]*in2[8],
        in1[2]*in2[0] + in1[5]*in2[3] + in1[8]*in2[6],
        in1[2]*in2[1] + in1[5]*in2[4] + in1[8]*in2[7],
        in1[2]*in2[2] + in1[5]*in2[5] + in1[8]*in2[8]]
    }

    pub fn matrix3_multiply_tps_second(in1: &[f64], in2: &[f64]) -> [f64; 9] {
        [in1[0]*in2[0] + in1[1]*in2[1] + in1[2]*in2[2],
        in1[0]*in2[3] + in1[1]*in2[4] + in1[2]*in2[5],
        in1[0]*in2[6] + in1[1]*in2[7] + in1[2]*in2[8],
        in1[3]*in2[0] + in1[4]*in2[1] + in1[5]*in2[2],
        in1[3]*in2[3] + in1[4]*in2[4] + in1[5]*in2[5],
        in1[3]*in2[6] + in1[4]*in2[7] + in1[5]*in2[8],
        in1[6]*in2[0] + in1[7]*in2[1] + in1[8]*in2[2],
        in1[6]*in2[3] + in1[7]*in2[4] + in1[8]*in2[5],
        in1[6]*in2[6] + in1[7]*in2[7] + in1[8]*in2[8]]
    }

    pub fn matrix3_inv(mat: &[f64]) -> [f64; 9] {
        // in = [a b c; d e f; g h i]
        let a = mat[4]*mat[8] - mat[5]*mat[7]; // (e*i - f*h)
        let d = mat[2]*mat[7] - mat[1]*mat[8]; // -(b*i - c*h)
        let g = mat[1]*mat[5] - mat[2]*mat[4]; // (b*f - c*e)
        let b = mat[5]*mat[6] - mat[3]*mat[8]; // -(d*i - f*g)
        let e = mat[0]*mat[8] - mat[2]*mat[6]; // (a*i - c*g)
        let h = mat[2]*mat[3] - mat[0]*mat[5]; // -(a*f - c*d)
        let c = mat[3]*mat[7] - mat[4]*mat[6]; // (d*h - e*g)
        let f = mat[1]*mat[6] - mat[0]*mat[7]; // -(a*h - b*g)
        let i = mat[0]*mat[4] - mat[1]*mat[3]; // (a*e - b*d)

        let det = mat[0]*a + mat[1]*b + mat[2]*c; // a*a + b*b + c*c;

        if det >= -EPS && det <= EPS {
            return [0.0; 9];
        }
        [a/det,d/det,g/det,b/det,e/det,h/det,c/det,f/det,i/det]
    }

    fn setup(&mut self) {
        assert!(self.coeffs.gyr_ts > 0.0);
        assert!(self.coeffs.acc_ts > 0.0);
        assert!(self.coeffs.mag_ts > 0.0);

        VQF::filter_coeffs(self.params.tau_acc, self.coeffs.acc_ts, &mut self.coeffs.acc_lp_b, &mut self.coeffs.acc_lp_a);

        self.coeffs.k_mag = VQF::gain_from_tau(self.params.tau_mag, self.coeffs.mag_ts);

        self.coeffs.bias_p0 = square(self.params.bias_sigma_init*100.0);
        // the system noise increases the variance from 0 to (0.1 °/s)^2 in biasForgettingTime seconds
        self.coeffs.bias_v = square(0.1*100.0)*self.coeffs.acc_ts/self.params.bias_forgetting_time;

        let p_motion = square(self.params.bias_sigma_motion*100.0);
        self.coeffs.bias_motion_w = square(p_motion) / self.coeffs.bias_v + p_motion;
        self.coeffs.bias_vertical_w = self.coeffs.bias_motion_w / self.params.bias_vertical_forgetting_factor.max(1e-10);

        let p_rest = square(self.params.bias_sigma_rest*100.0);
        self.coeffs.bias_rest_w = square(p_rest) / self.coeffs.bias_v + p_rest;

        VQF::filter_coeffs(self.params.rest_filter_tau, self.coeffs.gyr_ts, &mut self.coeffs.rest_gyr_lp_b, &mut self.coeffs.rest_gyr_lp_a);
        VQF::filter_coeffs(self.params.rest_filter_tau, self.coeffs.acc_ts, &mut self.coeffs.rest_acc_lp_b, &mut self.coeffs.rest_acc_lp_a);

        self.coeffs.k_mag_ref = VQF::gain_from_tau(self.params.mag_ref_tau, self.coeffs.mag_ts);
        if self.params.mag_current_tau > 0.0 {
            VQF::filter_coeffs(self.params.mag_current_tau, self.coeffs.mag_ts, &mut self.coeffs.mag_norm_dip_lp_b, &mut self.coeffs.mag_norm_dip_lp_a);
        } else {
            self.coeffs.mag_norm_dip_lp_b = [NAN; 3];
            self.coeffs.mag_norm_dip_lp_a = [NAN; 2];
        }

        self.reset_state();
    }
}

fn matrix3_multiply_vec(in_r: &[f64], in_v: &[f64]) -> [f64; 3] {
    [in_r[0]*in_v[0] + in_r[1]*in_v[1] + in_r[2]*in_v[2],
    in_r[3]*in_v[0] + in_r[4]*in_v[1] + in_r[5]*in_v[2],
    in_r[6]*in_v[0] + in_r[7]*in_v[1] + in_r[8]*in_v[2]]
}

fn integrate_gyr(gyr: &[f64], bias: &[f64], n: usize, ts: f64, out: &mut Vec<f64>) {
    let mut q = [1.0, 0.0, 0.0, 0.0];
    for i in 0..n {
        let gyr_no_bias = [gyr[3*i]-bias[3*i], gyr[3*i+1]-bias[3*i+1], gyr[3*i+2]-bias[3*i+2]];
        let gyrnorm = VQF::norm(&gyr_no_bias, 3);
        let angle = gyrnorm * ts;
        if gyrnorm > EPS {
            let c = (angle/2.0).cos();
            let s = (angle/2.0).sin()/gyrnorm;
            let gyr_step_quat = [c, s*gyr_no_bias[0], s*gyr_no_bias[1], s*gyr_no_bias[2]];
            q = VQF::quat_multiply(&q, &gyr_step_quat);
            VQF::normalize(&mut q, 4);
        }
        out.splice(4*i..4*i+4, q.iter().cloned());
    }
}

fn lowpass_butter_filtfilt(acc_i: &mut Vec<f64>, n: usize, ts: f64, tau: f64) {
    let mut b = [0f64; 3]; // check if everything compiles with float
    let mut a = [0f64; 2];
    let mut state = [NAN; 6];

    VQF::filter_coeffs(tau, ts, &mut b, &mut a);

    // forward filter
    for i in 0..n {
        let mut aout = [0f64; 3];
        VQF::filter_vec(&acc_i[3*i..3*i+3], 3, tau, ts, b, a, &mut state, &mut aout);
        acc_i.splice(3*i..3*i+3, aout.into_iter());
    }

    // backward filter
    for j in 0..3 {
        VQF::filter_initial_state(acc_i[3*n-3+j], b, a, &mut state[2*j..2*j+2]); // calculate initial state based on last sample
    }
    for i in (0..n).rev() {
        let mut aout = [0f64; 3];
        VQF::filter_vec(&acc_i[3*i..3*i+3], 3, tau, ts, b, a, &mut state, &mut aout);
        acc_i.splice(3*i..3*i+3, aout.into_iter());
    }
}

fn acc_correction(quat3d: &[f64], acc_i: &[f64], n: usize, quat6d: &mut Vec<f64>) {
    let mut acc_quat = [1.0, 0.0, 0.0, 0.0];

    for i in 0..n {
        // transform acc from inertial frame to 6D earth frame and normalize
        let mut acc_earth = VQF::quat_rotate(&acc_quat, &acc_i[3*i..3*i+3]);
        VQF::normalize(&mut acc_earth, 3);

        // inclination correction
        let q_w = ((acc_earth[2]+1.0)/2.0).sqrt();
        let acc_corr_quat = if q_w > 1e-6 {
            [q_w, 0.5*acc_earth[1]/q_w, -0.5*acc_earth[0]/q_w, 0.0]
        } else {
            // to avoid numeric issues when acc is close to [0 0 -1], i.e. the correction step is close (<= 0.00011°) to 180°:
            [0.0, 1.0, 0.0, 0.0]
        };
        acc_quat = VQF::quat_multiply(&acc_corr_quat, &acc_quat);
        VQF::normalize(&mut acc_quat, 4);

        // calculate output quaternion
        let qtemp = VQF::quat_multiply(&acc_quat, &quat3d[4*i..4*i+4]);
        quat6d.splice(4*i..4*i+4, qtemp.into_iter());
    }
}

fn calculate_delta(quat6d: &[f64], mag: &[f64], n: usize, delta: &mut Vec<f64>) {
    for i in 0..n {
        // bring magnetometer measurement into 6D earth frame
        let mag_earth = VQF::quat_rotate(&quat6d[4*i..4*i+4], &mag[3*i..3*i+3]);

        // calculate disagreement angle based on current magnetometer measurement
        delta[i] = mag_earth[0].atan2(mag_earth[1]);
    }
}

fn filter_delta(mag_dist: &[bool], n: usize, ts: f64, params: &VQFParams, backward: bool, delta: &mut Vec<f64>) {
    let mut d = if backward { delta[n-1] } else { delta[0] };
    let k_mag = VQF::gain_from_tau(params.tau_mag, ts);
    let mut k_mag_init: f64 = 1.0;
    let mut mag_reject_t: f64 = 0.0;

    for i in 0..n {
        let j= if backward { n-i-1 } else { i };
        let mut dis_angle = delta[j] - d;

        // make sure the disagreement angle is in the range [-pi, pi]
        if dis_angle > M_PI {
            dis_angle -= 2.0*M_PI;
        } else if dis_angle < -M_PI {
            dis_angle += 2.0*M_PI;
        }

        let mut k = k_mag;

        if params.mag_dist_rejection_enabled {
            // magnetic disturbance rejection
            if mag_dist[j] {
                if mag_reject_t <= params.mag_max_rejection_time {
                    mag_reject_t += ts;
                    k = 0.0;
                } else {
                    k /= params.mag_rejection_factor;
                }
            } else {
                mag_reject_t = (mag_reject_t - params.mag_rejection_factor*ts).max(0.0);
            }
        }

        // ensure fast initial convergence
        if k_mag_init.abs() != 0.0 {
            // make sure that the gain k is at least 1/N, N=1,2,3,... in the first few samples
            if k < k_mag_init {
                k = k_mag_init;
            }

            // iterative expression to calculate 1/N
            k_mag_init = k_mag_init/(k_mag_init+1.0);

            // disable if t > tauMag
            if k_mag_init*params.tau_mag < ts {
                k_mag_init = 0.0;
            }
        }

        // first-order filter step
        d += k*dis_angle;

        // make sure delta is in the range [-pi, pi]
        if d > M_PI {
            d -= 2.0*M_PI;
        } else if d < -M_PI {
            d += 2.0*M_PI;
        }

        // write output back into delta array
        delta[j] = d;
    }
}

pub fn offline_vqf(gyr: &[f64], acc: &[f64], mag: Option<&[f64]>, n: usize, ts: f64, params: VQFParams, quat6d: &mut Vec<f64>,
    mut out9d: Option<&mut Vec<f64>>, mut out_delta: Option<&mut Vec<f64>>, bias: &mut Vec<f64>, mut out_bias_sigma: Option<&mut Vec<f64>>,
    mut out_rest: Option<&mut Vec<bool>>, mut out_mag_dist: Option<&mut Vec<bool>>) {

    if quat6d.len() < n*4 { quat6d.resize(n*4, 0f64); }
    if bias.len()   < n*3 { bias  .resize(n*3, 0f64); }
    if mag.is_some() {
        let delta = out_delta.as_mut().unwrap();
        let mag_dist = out_mag_dist.as_mut().unwrap();
        let quat9d = out9d.as_mut().unwrap();
        if delta   .len() < n { delta.resize(n, 0f64); }
        if mag_dist.len() < n { mag_dist.resize(n, false); }
        if quat9d  .len() < n*4 { quat9d.resize(n*4, 0f64); }
    }

    // run real-time VQF implementation in forward direction
    let mut vqf = VQF::vqf(Some(params.clone()), ts, 0.0, 0.0);

    let mut bias_p_inv1 = vec![0f64; n*9];

    for i in 0..n {
        let g = &gyr[3*i..3*i+3];
        let a = &acc[3*i..3*i+3];
        if let Some(mag_vec) = mag {
            let m = &mag_vec[3*i..3*i+3];
            vqf.update(g, a, Some(m));
        } else {
            vqf.update(g, a, None);
        }
        if let Some(rest) = out_rest.as_mut() {
            rest[i] = vqf.get_rest_detected();
        }
        if let Some(mag_dist) = out_mag_dist.as_mut() {
            mag_dist[i] = vqf.get_mag_dist_detected();
        }
        bias.splice(3*i..3*i+3, vqf.get_bias_estimate().0.into_iter());
        bias_p_inv1.splice(9*i..9*i+9, VQF::matrix3_inv(&vqf.get_state().bias_p).into_iter());
    }


    // run real-time VQF implementation in backward direction
    vqf.reset_state();
    for i in (0..n).rev() {
        let temp_gyr = [-gyr[3*i], -gyr[3*i+1], -gyr[3*i+2]];
        let a = &acc[3*i..3*i+3];
        if let Some(mag_vec) = mag {
            let m = &mag_vec[3*i..3*i+3];
            vqf.update(&temp_gyr, a, Some(m));
        } else {
            vqf.update(&temp_gyr, a, None);
        }
        if let Some(rest) = out_rest.as_mut() {
            rest[i] = rest[i] || vqf.get_rest_detected();
        }
        if let Some(mag_dist) = out_mag_dist.as_mut() {
            mag_dist[i] = mag_dist[i] && vqf.get_mag_dist_detected();
        }

        let mut bias2 = vqf.get_bias_estimate().0;
        let bias_p_inv2 = VQF::matrix3_inv(&vqf.get_state().bias_p);

        // determine bias estimate by averaging both estimates via the covariances
        // P_1^-1 * b_1
        bias.splice(3*i..3*i+3, matrix3_multiply_vec(&bias_p_inv1[9*i..9*i+9], &bias[3*i..3*i+3]).into_iter());
        // P_2^-1 * b_2
        bias2 = matrix3_multiply_vec(&bias_p_inv2, &bias2);
        // P_1^-1 * b_1 - P_2^-1 * b_2
        bias[3*i] -= bias2[0];
        bias[3*i+1] -= bias2[1];
        bias[3*i+2] -= bias2[2];
        // (P_1^-1 + P_2^-1)^-1
        for j in 0..9 {
            bias_p_inv1[9*i+j] += bias_p_inv2[j];
        }
        bias_p_inv1.splice(9*i..9*i+9, VQF::matrix3_inv(&bias_p_inv1[9*i..9*i+9]).into_iter());
        // (P_1^-1 + P_2^-1)^-1 * (P_1^-1 * b_1 - P_2^-1 * b_2)
        bias.splice(3*i..3*i+3, matrix3_multiply_vec(&bias_p_inv1[9*i..9*i+9], &bias[3*i..3*i+3]).into_iter());
        // determine bias estimation uncertainty based on new covariance (P_1^-1 + P_2^-1)^-1
        // (cf. VQF::getBiasEstimate)
        if let Some(ref mut bias_sigma) = out_bias_sigma {
            let sum1 = bias_p_inv1[9*i+0].abs() + bias_p_inv1[9*i+1].abs() + bias_p_inv1[9*i+2].abs();
            let sum2 = bias_p_inv1[9*i+3].abs() + bias_p_inv1[9*i+4].abs() + bias_p_inv1[9*i+5].abs();
            let sum3 = bias_p_inv1[9*i+6].abs() + bias_p_inv1[9*i+7].abs() + bias_p_inv1[9*i+8].abs();
            let p = sum1.max(sum2).max(sum3);
            bias_sigma[i] = (p.sqrt()*(M_PI/100.0/180.0)).min(params.bias_sigma_init);
        }
    }

    // perform gyroscope integration
    let mut quat3d = vec![0f64; n*4];
    integrate_gyr(gyr, bias, n, ts, &mut quat3d);

    // transform acc to inertial frame
    let mut acc_i = Vec::<f64>::with_capacity(n as usize * 3);
    for i in 0..n {
        acc_i.extend(VQF::quat_rotate(&quat3d[4*i..4*i+4], &acc[3*i..3*i+3]).into_iter());
    }

    // filter acc in inertial frame
    lowpass_butter_filtfilt(&mut acc_i, n, ts, params.tau_acc);

    // inclination correction
    acc_correction(&quat3d, &acc_i, n, quat6d);

    // heading correction
    if let Some(mag) = mag {
        let delta = out_delta.as_mut().unwrap();
        let mag_dist = out_mag_dist.as_mut().unwrap();
        let quat9d = out9d.as_mut().unwrap();
        calculate_delta(quat6d, mag, n, delta);
        filter_delta(mag_dist, n, ts, &params, false, delta); // forward direction
        filter_delta(mag_dist, n, ts, &params, true, delta); // backward direction

        for i in 0..n  {
            quat9d.splice(4*i..4*i+4, VQF::quat_apply_delta(&quat6d[4*i..4*i+4], delta[i]).into_iter());
        }

    }
}
