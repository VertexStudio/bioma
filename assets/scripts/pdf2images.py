import json
import os
import sys
from PIL import Image, ImageDraw
import fitz  # PyMuPDF

# curl -X POST -F 'file=@/home/vertex/Downloads/4586eed3draft.pdf' localhost:5060 -o 4586eed3draft.jso

def extract_table_images(pdf_path):
    # Infer JSON path
    json_path = os.path.splitext(pdf_path)[0] + '.json'
    
    # Create output directory
    output_dir = os.path.splitext(pdf_path)[0]
    os.makedirs(output_dir, exist_ok=True)

    # Load JSON data
    with open(json_path, 'r') as f:
        data = json.load(f)

    # Open the PDF
    doc = fitz.open(pdf_path)

    # Process each box in the JSON data
    for box in data:
        if box.get('type') == 'Table':
            # Generate a unique name for the image
            unique_name = f"page_{box['page_number']}_{box['left']}_{box['top']}"
            
            # Get the page
            page = doc[box['page_number'] - 1]  # PDF pages are 0-indexed
            
            # Get the page dimensions
            page_rect = page.rect
            
            # Calculate the scaling factor
            scale_x = box['page_width'] / page_rect.width
            scale_y = box['page_height'] / page_rect.height
            
            # Calculate the table rectangle
            table_rect = fitz.Rect(
                box['left'] / scale_x,
                box['top'] / scale_y,
                (box['left'] + box['width']) / scale_x,
                (box['top'] + box['height']) / scale_y
            )
            
            # Render the page to an image
            pix = page.get_pixmap(matrix=fitz.Matrix(2, 2), clip=table_rect)
            img = Image.frombytes("RGB", [pix.width, pix.height], pix.samples)
            
            # Add a border to the image
            draw = ImageDraw.Draw(img)
            draw.rectangle([0, 0, img.width-1, img.height-1], outline="red", width=2)
            
            # Save the image
            img_path = os.path.join(output_dir, f"{unique_name}.png")
            img.save(img_path)
            print(f"Saved table image: {img_path}")

    # Close the PDF
    doc.close()

if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python pdf2images.py <path_to_pdf_file>")
        sys.exit(1)
    
    pdf_file = sys.argv[1]
    extract_table_images(pdf_file)