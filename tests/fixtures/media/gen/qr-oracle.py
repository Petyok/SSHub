import sys,glob,os
import cv2, zxingcpp
for p in sorted(glob.glob(sys.argv[1]+'/*.pgm')):
    im=cv2.imread(p,cv2.IMREAD_GRAYSCALE)
    z=len(zxingcpp.read_barcodes(im))
    try:
        v,_,_=cv2.QRCodeDetector().detectAndDecode(im); c=1 if v else 0
    except Exception: c=0
    print("RESULT",os.path.basename(p),"zxing=%d"%z,"opencv=%d"%c)
