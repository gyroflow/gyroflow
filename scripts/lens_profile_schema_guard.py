#!/usr/bin/env python3
import json
import sys
import os

def validate_lens_profile(filepath):
    try:
        with open(filepath, 'r') as f:
            data = json.load(f)
            
        issues = []
        
        # Check required root fields
        if 'camera_brand' not in data or not data['camera_brand']:
            issues.append("Missing or empty 'camera_brand'.")
            
        if 'camera_model' not in data or not data['camera_model']:
            issues.append("Missing or empty 'camera_model'.")
            
        if 'lens_model' not in data or not data['lens_model']:
            issues.append("Missing or empty 'lens_model'.")
            
        if 'lens_params' not in data:
            issues.append("Missing 'lens_params' section.")
        elif not isinstance(data['lens_params'], dict):
            issues.append("'lens_params' must be an object.")
        else:
            if 'focal_length' not in data['lens_params']:
                issues.append("Missing 'focal_length' inside 'lens_params'.")
                
        if issues:
            return False, issues
            
        return True, []
        
    except json.JSONDecodeError:
        return False, ["Invalid JSON format."]
    except Exception as e:
        return False, [str(e)]

def main():
    if len(sys.argv) < 2:
        print("Usage: python3 lens_profile_schema_guard.py <path_to_lens_profile.json>")
        sys.exit(1)
        
    target_file = sys.argv[1]
    
    if not os.path.exists(target_file):
        print(f"File not found: {target_file}")
        sys.exit(1)
        
    print(f"Validating {target_file}...")
    valid, issues = validate_lens_profile(target_file)
    
    if valid:
        print("PASS: Lens profile meets requirements.")
        sys.exit(0)
    else:
        print("FAIL: Lens profile rejected due to the following issues:")
        for issue in issues:
            print(f" - {issue}")
        sys.exit(1)

if __name__ == '__main__':
    main()
