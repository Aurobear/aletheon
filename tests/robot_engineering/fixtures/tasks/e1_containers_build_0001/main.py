"""Sample application for the container build task."""
import numpy as np

def process_data(samples):
    return np.mean(samples), np.std(samples)

if __name__ == "__main__":
    data = [1.0, 2.0, 3.0, 4.0, 5.0]
    mean, std = process_data(data)
    print(f"Mean: {mean}, Std: {std}")
