"""Synthetic PID controller with anti-windup — FIXED.

Uses back-calculation anti-windup to clamp integral term when output saturates.
"""

class PIDController:
    def __init__(self, kp: float, ki: float, kd: float, output_min: float = -100.0, output_max: float = 100.0):
        self.kp = kp
        self.ki = ki
        self.kd = kd
        self.output_min = output_min
        self.output_max = output_max
        self.integral = 0.0
        self.prev_error = 0.0

    def update(self, setpoint: float, measurement: float, dt: float) -> float:
        error = setpoint - measurement
        self.integral += error * dt
        derivative = (error - self.prev_error) / dt if dt > 0 else 0.0
        output = self.kp * error + self.ki * self.integral + self.kd * derivative
        # Clamp output with anti-windup
        if output > self.output_max:
            output = self.output_max
            # Back-calculate integral to prevent windup
            if self.ki != 0:
                self.integral = (output - self.kp * error - self.kd * derivative) / self.ki
        elif output < self.output_min:
            output = self.output_min
            if self.ki != 0:
                self.integral = (output - self.kp * error - self.kd * derivative) / self.ki
        self.prev_error = error
        return output

    def reset(self):
        self.integral = 0.0
        self.prev_error = 0.0
