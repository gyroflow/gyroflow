// Ported from https://github.com/ccny-ros-pkg/imu_tools/blob/indigo/imu_complementary_filter/src/complementary_filter.cpp
// Original author: Roberto G. Valenti <robertogl.valenti@gmail.com>
// With modifications by Elvin

// Based on "Keeping a Good Attitude: A Quaternion-Based Orientation Filter for IMUs and MARGs"
// https://www.mdpi.com/1424-8220/15/8/19302


#[allow(dead_code)]
const GRAVITY: f64 = 9.81;
// Bias estimation steady state thresholds
const ANGULAR_VELOCITY_THRESHOLD: f64 = 0.01; // 1,7 deg/s
const ACCELERATION_THRESHOLD: f64 = 0.1;
const DELTA_ANGULAR_VELOCITY_THRESHOLD: f64 = 0.01;
const DELTA_ACCELERATION_THRESHOLD: f64 = 0.05; // Compare current to IIR result
const GRAV_AUTOSCALE_THRESHOLD: f64 = 1.0; // apply autoscaling when steady state and acceleration within 1 m/s^2 of GRAVITY
const ACC_FILT_TIMECONSTANT: f64 = 0.1; // slight iir filtering to clear spikes.
const GRAV_AUTOSCALE_ALPHA: f64 = 0.005;
const STEADY_WAIT_THRESHOLD: f64 = 0.2;

pub struct ComplementaryFilterV2 {
    // Gain parameter for the complementary filter, belongs in [0, 1].
    pub gain_acc: f64,
    pub gain_mag: f64,

    // Bias estimation gain parameter, belongs in [0, 1].
    pub bias_alpha: f64,

    pub do_bias_estimation: bool,
    pub do_adaptive_gain: bool,
    pub do_gravity_autoscale: bool,
    pub gravity: f64,

    pub initialized: bool,
    pub steady_state: bool,
    pub partial_steady_state: bool,

    // The orientation as a Hamilton quaternion (q.0 is the scalar). Represents the orientation of the fixed frame wrt the body frame.
    q: (f64, f64, f64, f64),

    // filtered acceleration
    a_filt: (f64, f64, f64),
    a_prev: (f64, f64, f64),
    prev_gain_acc: f64,

    // Prev angular velocities;
    w_prev: (f64, f64, f64),

    // Bias in angular velocities;
    w_bias: (f64, f64, f64),
    time: f64,
    time_steady: f64,

    // initial time with higher accel gain
    initial_settle_time: f64,
}

impl Default for ComplementaryFilterV2 {
    fn default() -> Self {
        Self {
            gain_acc: 0.0004,
            prev_gain_acc: 0.0,
            gain_mag: 0.0004,
            bias_alpha: 0.001,
            do_bias_estimation: true,
            do_adaptive_gain: true,
            do_gravity_autoscale: true,
            gravity: 9.81,
            initialized: false,
            steady_state: false,
            partial_steady_state: false,
            q: (1.0, 0.0, 0.0, 0.0),
            a_filt: (0.0, 0.0, 0.0),
            a_prev: (0.0, 0.0, 0.0),
            w_prev: (0.0, 0.0, 0.0),
            w_bias: (0.0, 0.0, 0.0),
            time: 0.0,
            time_steady: 0.0,
            initial_settle_time: 2.0,
        }
    }
}

#[allow(dead_code)]
impl ComplementaryFilterV2 {
    pub fn set_gain_acc(&mut self, gain: f64) -> bool {
        if (0.0..=1.0).contains(&gain) {
            self.gain_acc = gain;
            true
        } else {
            false
        }
    }
    pub fn set_gain_mag(&mut self, gain: f64) -> bool {
        if (0.0..=1.0).contains(&gain) {
            self.gain_mag = gain;
            true
        } else {
            false
        }
    }
    pub fn set_bias_alpha(&mut self, bias_alpha: f64) -> bool {
        if (0.0..=1.0).contains(&bias_alpha) {
            self.bias_alpha = bias_alpha;
            true
        } else {
            false
        }
    }

    pub fn set_initial_settle_time(&mut self, settle_time: f64) {
        self.initial_settle_time = settle_time;
    }

    // When the filter is in the steady state, bias estimation will occur (if the parameter is enabled).
    pub fn get_steady_state(&self) -> bool {
        self.steady_state
    }

    // Set the orientation, as a Hamilton Quaternion, of the body frame wrt the fixed frame.
    pub fn set_orientation(&mut self, q0: f64, q1: f64, q2: f64, q3: f64) {
        self.q = invert_quaternion(q0, q1, q2, q3);
    }

    // Get the orientation, as a Hamilton Quaternion, of the body frame wrt the fixed frame.
    pub fn get_orientation(&self) -> (f64, f64, f64, f64) {
        invert_quaternion(self.q.0, self.q.1, self.q.2, self.q.3)
    }

    // Update from accelerometer and gyroscope data.
    // [ax, ay, az]: Normalized gravity vector.
    // [wx, wy, wz]: Angular veloctiy, in rad / s.
    // dt: time delta, in seconds.
    pub fn update(&mut self, ax: f64, ay: f64, az: f64, wx: f64, wy: f64, wz: f64, dt: f64) {
        if !self.initialized {
            // First time - ignore prediction:
            self.q = self.get_measurement(ax, ay, az);
            self.a_filt = (ax, ay, az);
            self.a_prev = (ax, ay, az);
            self.initialized = true;
            return;
        }

        let (axf, ayf, azf) = self.filter_acc(ax, ay, az, dt);
        self.steady_state = self.check_state(ax, ay, az, wx, wy, wz);
        self.time_steady = if self.steady_state { self.time_steady + dt } else { 0.0 };

        // Bias estimation.
        if self.do_bias_estimation {
            self.update_biases(axf, ayf, azf, wx, wy, wz);
        }

        if self.do_gravity_autoscale {
            self.autoscale_gravity(axf, ayf, azf)
        }

        // Prediction.
        let pred = self.get_prediction(wx, wy, wz, dt);

        // Correction (from acc):
        // q_ = q_pred * [(1-gain) * qI + gain * dq_acc]
        // where qI = identity quaternion
        // filtered accel for correction to avoid jittery motion
        let mut dq_acc = self.get_acc_correction(axf, ayf, azf, pred.0, pred.1, pred.2, pred.3);

        let gain = self.get_adaptive_gain(self.gain_acc, axf, ayf, azf, dt);

        scale_quaternion(gain, &mut dq_acc.0, &mut dq_acc.1, &mut dq_acc.2, &mut dq_acc.3);

        self.q = quaternion_multiplication(pred.0, pred.1, pred.2, pred.3, dq_acc.0, dq_acc.1, dq_acc.2, dq_acc.3);

        normalize_quaternion(&mut self.q.0, &mut self.q.1, &mut self.q.2, &mut self.q.3);

        self.time += dt;
    }

    // Update from accelerometer, gyroscope, and magnetometer data.
    // [ax, ay, az]: Normalized gravity vector.
    // [wx, wy, wz]: Angular veloctiy, in rad / s.
    // [mx, my, mz]: Magnetic field, units irrelevant.
    // dt: time delta, in seconds.
    pub fn update_mag(&mut self, ax: f64, ay: f64, az: f64, wx: f64, wy: f64, wz: f64, mx: f64, my: f64, mz: f64, dt: f64) {
        if !self.initialized {
            // First time - ignore prediction:
            self.q = self.get_measurement_mag(ax, ay, az, mx, my, mz);
            self.a_filt = (ax, ay, az);
            self.a_prev = (ax, ay, az);
            self.initialized = true;
            return;
        }

        let (axf, ayf, azf) = self.filter_acc(ax, ay, az, dt);
        self.steady_state = self.check_state(ax, ay, az, wx, wy, wz);
        self.time_steady = if self.steady_state { self.time_steady + dt } else { 0.0 };

        // Bias estimation.
        if self.do_bias_estimation {
            self.update_biases(ax, ay, az, wx, wy, wz);
        }
        if self.do_gravity_autoscale {
            self.autoscale_gravity(axf, ayf, azf)
        }

        // Prediction.
        let pred = self.get_prediction(wx, wy, wz, dt);

        // Correction (from acc):
        // q_ = q_pred * [(1-gain) * qI + gain * dq_acc]
        // where qI = identity quaternion
        let mut dq_acc = self.get_acc_correction(ax, ay, az, pred.0, pred.1, pred.2, pred.3);

        let gain = self.get_adaptive_gain(self.gain_acc, axf, ayf, azf, dt);

        scale_quaternion(gain, &mut dq_acc.0, &mut dq_acc.1, &mut dq_acc.2, &mut dq_acc.3);

        let q_temp = quaternion_multiplication(pred.0, pred.1, pred.2, pred.3, dq_acc.0, dq_acc.1, dq_acc.2, dq_acc.3);

        // Correction (from mag):
        // q_ = q_temp * [(1-gain) * qI + gain * dq_mag]
        // where qI = identity quaternion
        let mut dq_mag = self.get_mag_correction(mx, my, mz, q_temp.0, q_temp.1, q_temp.2, q_temp.3);

        scale_quaternion(self.gain_mag, &mut dq_mag.0, &mut dq_mag.1, &mut dq_mag.2, &mut dq_mag.3);

        self.q = quaternion_multiplication(q_temp.0, q_temp.1, q_temp.2, q_temp.3, dq_mag.0, dq_mag.1, dq_mag.2, dq_mag.3);

        normalize_quaternion(&mut self.q.0, &mut self.q.1, &mut self.q.2, &mut self.q.3);
    }

    fn filter_acc(&mut self, ax: f64, ay: f64, az: f64, dt: f64) -> (f64, f64, f64) {
        // simple IIR filter to acceleration
        let iir_alpha = 1.0 - (-dt/ACC_FILT_TIMECONSTANT).exp();
        self.a_filt.0 = iir_alpha * ax + (1.0-iir_alpha) * self.a_filt.0;
        self.a_filt.1 = iir_alpha * ay + (1.0-iir_alpha) * self.a_filt.1;
        self.a_filt.2 = iir_alpha * az + (1.0-iir_alpha) * self.a_filt.2;
        self.a_filt.clone()
    }

    fn update_biases(&mut self, _ax: f64, _ay: f64, _az: f64, wx: f64, wy: f64, wz: f64) {
        if self.time_steady > STEADY_WAIT_THRESHOLD {
            self.w_bias.0 += self.bias_alpha * (wx - self.w_bias.0);
            self.w_bias.1 += self.bias_alpha * (wy - self.w_bias.1);
            self.w_bias.2 += self.bias_alpha * (wz - self.w_bias.2);
        }
    }

    fn autoscale_gravity(&mut self, _ax: f64, _ay: f64, _az: f64) {
        //self.steady_state = self.check_state(ax, ay, az, wx, wy, wz);

        if self.partial_steady_state {
            // autoscale with filtered acceleration values
            let acc_magnitude = (self.a_filt.0*self.a_filt.0 + self.a_filt.1*self.a_filt.1 + self.a_filt.2*self.a_filt.2).sqrt();
            // Within reasonable amount of true acceleration
            if (acc_magnitude - GRAVITY).abs() < GRAV_AUTOSCALE_THRESHOLD {
                self.gravity = self.gravity * (1.0-GRAV_AUTOSCALE_ALPHA) + GRAV_AUTOSCALE_ALPHA * acc_magnitude;
            }
        }

    }

    fn check_state(&mut self, ax: f64, ay: f64, az: f64, wx: f64, wy: f64, wz: f64) -> bool {
        let acc_magnitude = (ax*ax + ay*ay + az*az).sqrt();

        let acc_th = (acc_magnitude - self.gravity).abs() < ACCELERATION_THRESHOLD;
        let acc_component_steady = (ax - self.a_filt.0).abs() < DELTA_ACCELERATION_THRESHOLD ||
                                        (ay - self.a_filt.1).abs() < DELTA_ACCELERATION_THRESHOLD ||
                                        (az - self.a_filt.2).abs() < DELTA_ACCELERATION_THRESHOLD;
        let acc_delta_th = (ax - self.a_prev.0).abs() < DELTA_ACCELERATION_THRESHOLD ||
                                (ay - self.a_prev.1).abs() < DELTA_ACCELERATION_THRESHOLD ||
                                (az - self.a_prev.2).abs() < DELTA_ACCELERATION_THRESHOLD;
        let gyro_delta_th = (wx - self.w_prev.0).abs() < DELTA_ANGULAR_VELOCITY_THRESHOLD ||
                                 (wy - self.w_prev.1).abs() < DELTA_ANGULAR_VELOCITY_THRESHOLD ||
                                 (wz - self.w_prev.2).abs() < DELTA_ANGULAR_VELOCITY_THRESHOLD;
        let gyro_th =    (wx - self.w_bias.0).abs() < ANGULAR_VELOCITY_THRESHOLD ||
                              (wy - self.w_bias.1).abs() < ANGULAR_VELOCITY_THRESHOLD ||
                              (wz - self.w_bias.2).abs() < ANGULAR_VELOCITY_THRESHOLD;

        self.w_prev = (wx, wy, wz);
        self.a_prev = (ax, ay, az);

        // satisfy conditions for correcting acceleration
        self.partial_steady_state = acc_component_steady && acc_delta_th && gyro_delta_th && gyro_th;

        // satisfy all thresholds for stationary
        acc_th && acc_component_steady && acc_delta_th && gyro_delta_th && gyro_th
    }

    fn get_prediction(&self, wx: f64, wy: f64, wz: f64, dt: f64) -> (f64, f64, f64, f64) {
        let wx_unb = wx - self.w_bias.0;
        let wy_unb = wy - self.w_bias.1;
        let wz_unb = wz - self.w_bias.2;

        let mut q0_pred = self.q.0 + 0.5*dt*( wx_unb*self.q.1 + wy_unb*self.q.2 + wz_unb*self.q.3);
        let mut q1_pred = self.q.1 + 0.5*dt*(-wx_unb*self.q.0 - wy_unb*self.q.3 + wz_unb*self.q.2);
        let mut q2_pred = self.q.2 + 0.5*dt*( wx_unb*self.q.3 - wy_unb*self.q.0 - wz_unb*self.q.1);
        let mut q3_pred = self.q.3 + 0.5*dt*(-wx_unb*self.q.2 + wy_unb*self.q.1 - wz_unb*self.q.0);

        normalize_quaternion(&mut q0_pred, &mut q1_pred, &mut q2_pred, &mut q3_pred);

        (q0_pred, q1_pred, q2_pred, q3_pred)
    }

    fn get_measurement(&mut self, mut ax: f64, mut ay: f64, mut az: f64) -> (f64, f64, f64, f64) {
        // q_acc is the quaternion obtained from the acceleration vector representing
        // the orientation of the Global frame wrt the Local frame with arbitrary yaw
        // (intermediary frame). q3_acc is defined as 0.

        // Normalize acceleration vector.
        normalize_vector(&mut ax, &mut ay, &mut az);

        if az >= 0.0 {
            let q0_meas = ((az + 1.0) * 0.5).sqrt();
            (
                q0_meas,
                -ay / (2.0 * q0_meas),
                ax / (2.0 * q0_meas),
                0.0
            )
        } else {
            let x = ((1.0 - az) * 0.5).sqrt();
            (
                -ay / (2.0 * x),
                x,
                0.0,
                ax / (2.0 * x)
            )
        }
    }

    fn get_measurement_mag(&mut self, mut ax: f64, mut ay: f64, mut az: f64, mx: f64, my: f64, mz: f64) -> (f64, f64, f64, f64) {
        // q_acc is the quaternion obtained from the acceleration vector representing
        // the orientation of the Global frame wrt the Local frame with arbitrary yaw
        // (intermediary frame). q3_acc is defined as 0.
        // Normalize acceleration vector.
        normalize_vector(&mut ax, &mut ay, &mut az);

        let q_acc = if az >= 0.0 {
            let q0_acc = ((az + 1.0) * 0.5).sqrt();
            (
                q0_acc,
                -ay / (2.0 * q0_acc),
                ax / (2.0 * q0_acc),
                0.0
            )
        } else {
            let x = ((1.0 - az) * 0.5).sqrt();
            (
                -ay / (2.0 * x),
                x,
                0.0,
                ax / (2.0 * x)
            )
        };

        // [lx, ly, lz] is the magnetic field reading, rotated into the intermediary frame by the inverse of q_acc.
        // l = R(q_acc)^-1 m
        let lx = (q_acc.0 * q_acc.0 + q_acc.1 * q_acc.1 - q_acc.2 * q_acc.2) * mx + 2.0 * (q_acc.1 * q_acc.2) * my - 2.0 * (q_acc.0 * q_acc.2) * mz;
        let ly = 2.0 * (q_acc.1 * q_acc.2) * mx + (q_acc.0 * q_acc.0 - q_acc.1 * q_acc.1 + q_acc.2 * q_acc.2) * my + 2.0 * (q_acc.0 * q_acc.1) * mz;

        // q_mag is the quaternion that rotates the Global frame (North West Up) into the intermediary frame. q1_mag and q2_mag are defined as 0.
        let gamma = lx * lx + ly * ly;
        let beta = (gamma + lx * gamma.sqrt()).sqrt();
        let q0_mag = beta / ((2.0 * gamma).sqrt());
        let q3_mag = ly / (std::f64::consts::SQRT_2 * beta);

        // The quaternion multiplication between q_acc and q_mag represents the quaternion, orientation of the Global frame wrt the local frame.
        // q = q_acc times q_mag
        quaternion_multiplication(q_acc.0, q_acc.1, q_acc.2, q_acc.3,
                                  q0_mag, 0.0, 0.0, q3_mag)
        // (
        //    q_acc.0 * q0_mag,
        //    q_acc.1 * q0_mag + q_acc.2 * q3_mag,
        //    q_acc.2 * q0_mag - q_acc.1 * q3_mag,
        //    q0_acc * q3_mag
        // )
    }

    fn get_acc_correction(&mut self, mut ax: f64, mut ay: f64, mut az: f64, p0: f64, p1: f64, p2: f64, p3: f64) -> (f64, f64, f64, f64) {
        // Normalize acceleration vector.
        normalize_vector(&mut ax, &mut ay, &mut az);

        // Acceleration reading rotated into the world frame by the inverse predicted quaternion (predicted gravity):
        let g = rotate_vector_by_quaternion(ax, ay, az, p0, -p1, -p2, -p3);

        // Delta quaternion that rotates the predicted gravity into the real gravity:
        let dq0 =  ((g.2 + 1.0) * 0.5).sqrt();
        (
            dq0,
            -g.1 / (2.0 * dq0),
            g.0 / (2.0 * dq0),
            0.0
        )
    }

    fn get_mag_correction(&mut self, mx: f64, my: f64, mz: f64, p0: f64, p1: f64, p2: f64, p3: f64) -> (f64, f64, f64, f64) {
        // Magnetic reading rotated into the world frame by the inverse predicted quaternion:
        let l = rotate_vector_by_quaternion(mx, my, mz, p0, -p1, -p2, -p3);

        // Delta quaternion that rotates the l so that it lies in the xz-plane (points north):
        let gamma = l.0*l.0 + l.1*l.1;
        let beta = (gamma + l.0*gamma.sqrt()).sqrt();
        (
            beta / ((2.0 * gamma).sqrt()),
            0.0,
            0.0,
            l.1 / (std::f64::consts::SQRT_2 * beta)
        )
    }

    fn get_adaptive_gain(&mut self, alpha: f64, ax: f64, ay: f64, az: f64, dt: f64) -> f64 {

        if self.do_adaptive_gain {
            let a_mag = (ax * ax + ay * ay + az * az).sqrt();
            let w_mag = (self.w_prev.0*self.w_prev.0 + self.w_prev.1*self.w_prev.1 + self.w_prev.2*self.w_prev.2).sqrt();
            let error = (a_mag - self.gravity).abs() / self.gravity;

            let gain_iir_alpha = 1.0 - (-dt/0.15).exp(); // 0.15s time constant filtering for gain

            // scaling of 0.13 at error of 5% = 0.5 m/s^2
            // initial settle gain factor is constant followed by slope down to 1
            let new_gain = if self.time_steady > STEADY_WAIT_THRESHOLD {
                8.0 * alpha
            } else {
                (-40.0*error -1.0*w_mag).exp() * alpha * (if self.time < self.initial_settle_time { (15.0-self.time/self.initial_settle_time*14.0).max(8.0) } else { 1.0 })
            };
            // 1st order filter of gain when increasing
            let gain = if new_gain < self.prev_gain_acc { new_gain } else { gain_iir_alpha * new_gain + (1.0-gain_iir_alpha) * self.prev_gain_acc };
            self.prev_gain_acc = gain;
            gain

        } else {
            alpha
        }
      }
}


fn normalize_vector(x: &mut f64, y: &mut f64, z: &mut f64) {
    let norm = (*x**x + *y**y + *z**z).sqrt();
    if norm.is_finite() && norm != 0.0 {
        *x /= norm;
        *y /= norm;
        *z /= norm;
    }
}

fn normalize_quaternion(q0: &mut f64, q1: &mut f64, q2: &mut f64, q3: &mut f64) {
    let norm = (*q0**q0 + *q1**q1 + *q2**q2 + *q3**q3).sqrt();
    if norm.is_finite() && norm != 0.0 {
        *q0 /= norm;
        *q1 /= norm;
        *q2 /= norm;
        *q3 /= norm;
    }
}

fn invert_quaternion(q0: f64, q1: f64, q2: f64, q3: f64) -> (f64, f64, f64, f64) {
    // Assumes quaternion is normalized.
    (q0, -q1, -q2, -q3)
}

fn scale_quaternion(gain: f64, dq0: &mut f64, dq1: &mut f64, dq2: &mut f64, dq3: &mut f64) {
	if *dq0 < 0.0 { // 0.9
        // Slerp (Spherical linear interpolation):
        let angle = dq0.acos();
        let a = (angle * (1.0 - gain)).sin() / angle.sin();
        let b = (angle * gain).sin() / angle.sin();
        *dq0 = a + b * *dq0;
        *dq1 *= b;
        *dq2 *= b;
        *dq3 *= b;
    } else {
        // Lerp (Linear interpolation):
        *dq0 = (1.0 - gain) + gain * *dq0;
        *dq1 *= gain;
        *dq2 *= gain;
        *dq3 *= gain;
    }

    normalize_quaternion(dq0, dq1, dq2, dq3);
}

fn quaternion_multiplication(p0: f64, p1: f64, p2: f64, p3: f64, q0: f64, q1: f64, q2: f64, q3: f64) -> (f64, f64, f64, f64) {
    ( // r = p q
        p0*q0 - p1*q1 - p2*q2 - p3*q3,
        p0*q1 + p1*q0 + p2*q3 - p3*q2,
        p0*q2 - p1*q3 + p2*q0 + p3*q1,
        p0*q3 + p1*q2 - p2*q1 + p3*q0
    )
}

fn rotate_vector_by_quaternion(x: f64, y: f64, z: f64, q0: f64, q1: f64, q2: f64, q3: f64) -> (f64, f64, f64) {
    (
        (q0*q0 + q1*q1 - q2*q2 - q3*q3)*x + 2.0*(q1*q2 - q0*q3)*y + 2.0*(q1*q3 + q0*q2)*z,
        2.0*(q1*q2 + q0*q3)*x + (q0*q0 - q1*q1 + q2*q2 - q3*q3)*y + 2.0*(q2*q3 - q0*q1)*z,
        2.0*(q1*q3 - q0*q2)*x + 2.0*(q2*q3 + q0*q1)*y + (q0*q0 - q1*q1 - q2*q2 + q3*q3)*z
    )
}
